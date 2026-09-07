use crate::{job::Job, launch::LaunchSpec};
use tokio::io::AsyncRead;

pub type OutputPipe = Box<dyn AsyncRead + Unpin + Send>;

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        collections::BTreeMap,
        ffi::OsStr,
        fs::File,
        io,
        mem::{size_of, zeroed},
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, SetHandleInformation, GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT,
            INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::{
            Pipes::CreatePipe,
            Threading::{
                CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
                InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
                CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
                EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
                STARTUPINFOEXW,
            },
        },
    };

    // Win32 ProcThreadAttributeValue(JobList / HandleList, FALSE, TRUE, FALSE).
    const ATTRIBUTE_JOB_LIST: usize = 0x0002000d;
    const ATTRIBUTE_HANDLE_LIST: usize = 0x00020002;

    struct Handle(usize);
    impl Handle {
        fn new(handle: HANDLE) -> Self {
            Self(handle as usize)
        }
        fn raw(&self) -> HANDLE {
            self.0 as HANDLE
        }
        fn into_file(mut self) -> File {
            let handle = self.raw();
            self.0 = 0;
            unsafe { File::from_raw_handle(handle) }
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe {
                    CloseHandle(self.raw());
                }
            }
        }
    }

    struct Attributes {
        buffer: Vec<usize>,
        initialized: bool,
    }
    impl Attributes {
        fn new(count: u32) -> io::Result<Self> {
            let mut size = 0;
            unsafe {
                InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size);
            }
            if size == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut attributes = Self {
                buffer: vec![0; size.div_ceil(size_of::<usize>())],
                initialized: false,
            };
            if unsafe {
                InitializeProcThreadAttributeList(attributes.pointer(), count, 0, &mut size)
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            attributes.initialized = true;
            Ok(attributes)
        }
        fn pointer(&mut self) -> *mut std::ffi::c_void {
            self.buffer.as_mut_ptr().cast()
        }
        fn set(
            &mut self,
            attribute: usize,
            value: *const std::ffi::c_void,
            size: usize,
        ) -> io::Result<()> {
            if unsafe {
                UpdateProcThreadAttribute(
                    self.pointer(),
                    0,
                    attribute,
                    value,
                    size,
                    null_mut(),
                    null(),
                )
            } == 0
            {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
    impl Drop for Attributes {
        fn drop(&mut self) {
            if self.initialized {
                unsafe {
                    DeleteProcThreadAttributeList(self.pointer());
                }
            }
        }
    }

    pub struct NativeChild {
        handle: Handle,
        pub pid: u32,
        pub stdout: Option<OutputPipe>,
        pub stderr: Option<OutputPipe>,
    }

    impl NativeChild {
        pub fn spawn(spec: &LaunchSpec, job: &Job) -> Result<Self, String> {
            spawn(spec, job).map_err(|error| format!("创建进程失败：{error}"))
        }
        pub fn try_wait(&mut self) -> Result<Option<i32>, String> {
            match unsafe { WaitForSingleObject(self.handle.raw(), 0) } {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    if unsafe { GetExitCodeProcess(self.handle.raw(), &mut code) } == 0 {
                        Err(io::Error::last_os_error().to_string())
                    } else {
                        Ok(Some(code as i32))
                    }
                }
                _ => Err(io::Error::last_os_error().to_string()),
            }
        }
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    /// Use a kernel creation attribute, rather than assigning the job after
    /// spawn. Even a wrapper that immediately spawns children cannot escape.
    fn spawn(spec: &LaunchSpec, job: &Job) -> io::Result<NativeChild> {
        let executable = wide(spec.program.as_os_str());
        let directory = wide(spec.directory.as_os_str());
        let mut line = quote_windows_arg(&spec.program.to_string_lossy());
        if let Some(raw) = &spec.raw_args {
            line.push(' ');
            line.push_str(raw);
        } else {
            for argument in &spec.args {
                line.push(' ');
                line.push_str(&quote_windows_arg(argument));
            }
        }
        let mut line = wide(OsStr::new(&line));
        if line.len() > 32767 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "命令超过 Windows 的 32767 字符限制",
            ));
        }
        let mut environment = environment_block(&spec.environment);
        let mut attributes = Attributes::new(if spec.hide_window { 2 } else { 1 })?;
        let job_handle = job.raw_handle();
        attributes.set(
            ATTRIBUTE_JOB_LIST,
            &job_handle as *const _ as _,
            size_of::<HANDLE>(),
        )?;

        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = attributes.pointer();
        let mut stdout_read = None;
        let mut stderr_read = None;
        let mut inherited = Vec::new();
        let mut inherited_handles = Vec::new();
        if spec.hide_window {
            let (read, write) = pipe()?;
            stdout_read = Some(read);
            startup.StartupInfo.hStdOutput = write.raw();
            inherited.push(write);
            let (read, write) = pipe()?;
            stderr_read = Some(read);
            startup.StartupInfo.hStdError = write.raw();
            inherited.push(write);
            let security = security_attributes();
            let nul = wide(OsStr::new("NUL"));
            let handle = unsafe {
                CreateFileW(
                    nul.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &security,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let input = Handle::new(handle);
            startup.StartupInfo.hStdInput = input.raw();
            inherited.push(input);
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            inherited_handles.extend(inherited.iter().map(Handle::raw));
            attributes.set(
                ATTRIBUTE_HANDLE_LIST,
                inherited_handles.as_ptr().cast(),
                inherited_handles.len() * size_of::<HANDLE>(),
            )?;
        }
        let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
        let flags = CREATE_UNICODE_ENVIRONMENT
            | EXTENDED_STARTUPINFO_PRESENT
            | if spec.hide_window {
                CREATE_NO_WINDOW
            } else {
                CREATE_NEW_CONSOLE
            };
        let created = unsafe {
            CreateProcessW(
                executable.as_ptr(),
                line.as_mut_ptr(),
                null(),
                null(),
                i32::from(spec.hide_window),
                flags,
                environment.as_mut_ptr().cast(),
                directory.as_ptr(),
                &startup.StartupInfo,
                &mut process,
            )
        };
        if created == 0 {
            return Err(io::Error::last_os_error());
        }
        let handle = Handle::new(process.hProcess);
        let _thread = Handle::new(process.hThread);
        // Parent copies of the write ends are dropped here; EOF then follows
        // the actual descendant lifetime, not the launcher lifetime.
        drop(inherited);
        let stdout = stdout_read
            .map(|handle| Box::new(tokio::fs::File::from_std(handle.into_file())) as OutputPipe);
        let stderr = stderr_read
            .map(|handle| Box::new(tokio::fs::File::from_std(handle.into_file())) as OutputPipe);
        Ok(NativeChild {
            handle,
            pid: process.dwProcessId,
            stdout,
            stderr,
        })
    }

    fn security_attributes() -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        }
    }

    fn pipe() -> io::Result<(Handle, Handle)> {
        let mut read = null_mut();
        let mut write = null_mut();
        let security = security_attributes();
        if unsafe { CreatePipe(&mut read, &mut write, &security, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let read = Handle::new(read);
        let write = Handle::new(write);
        if unsafe { SetHandleInformation(read.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((read, write))
    }

    fn environment_block(extra: &[(String, String)]) -> Vec<u16> {
        let mut variables = BTreeMap::new();
        for (key, value) in std::env::vars_os() {
            variables.insert(key.to_string_lossy().to_uppercase(), (key, value));
        }
        for (key, value) in extra {
            variables.insert(key.to_uppercase(), (key.into(), value.into()));
        }
        let mut result = Vec::new();
        for (_, (key, value)) in variables {
            result.extend(key.encode_wide());
            result.push(b'=' as u16);
            result.extend(value.encode_wide());
            result.push(0);
        }
        result.push(0);
        result
    }

    pub fn quote_windows_arg(argument: &str) -> String {
        let mut result = String::from("\"");
        let mut slashes = 0;
        for character in argument.chars() {
            if character == '\\' {
                slashes += 1;
                continue;
            }
            if character == '"' {
                result.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
            } else {
                result.extend(std::iter::repeat_n('\\', slashes));
            }
            slashes = 0;
            result.push(character);
        }
        result.extend(std::iter::repeat_n('\\', slashes * 2));
        result.push('"');
        result
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::process::Stdio;
    pub struct NativeChild {
        child: tokio::process::Child,
        pub pid: u32,
        pub stdout: Option<OutputPipe>,
        pub stderr: Option<OutputPipe>,
    }
    impl NativeChild {
        pub fn spawn(spec: &LaunchSpec, job: &Job) -> Result<Self, String> {
            use std::os::unix::process::CommandExt;
            let mut command = tokio::process::Command::new(&spec.program);
            command
                .args(&spec.args)
                .current_dir(&spec.directory)
                .envs(spec.environment.clone())
                .kill_on_drop(true);
            command.as_std_mut().process_group(0);
            if spec.hide_window {
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
            }
            let mut child = command.spawn().map_err(|e| e.to_string())?;
            let pid = child.id().ok_or("无法读取子进程 PID")?;
            job.assign_pid(pid);
            let stdout = child.stdout.take().map(|pipe| Box::new(pipe) as OutputPipe);
            let stderr = child.stderr.take().map(|pipe| Box::new(pipe) as OutputPipe);
            Ok(Self {
                child,
                pid,
                stdout,
                stderr,
            })
        }
        pub fn try_wait(&mut self) -> Result<Option<i32>, String> {
            self.child
                .try_wait()
                .map(|status| status.map(|status| status.code().unwrap_or(-1)))
                .map_err(|e| e.to_string())
        }
    }
}

pub use platform::NativeChild;

#[cfg(all(test, windows))]
mod tests {
    use super::platform::quote_windows_arg;
    use crate::launch::split_windows_arguments;
    #[test]
    fn windows_argument_serialization_round_trips() {
        for value in [
            "",
            "alpha beta",
            "中文",
            r#"say "hello""#,
            r"C:\data\",
            r#"C:\some dir\"quoted""#,
        ] {
            assert_eq!(
                split_windows_arguments(&quote_windows_arg(value)).unwrap(),
                [value]
            );
        }
    }
}
