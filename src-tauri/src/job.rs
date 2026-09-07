#[cfg(windows)]
mod platform {
    use std::{
        io,
        mem::{size_of, zeroed},
        ptr::null,
    };
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::JobObjects::{
            CreateJobObjectW, JobObjectBasicAccountingInformation,
            JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
            TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    /// One kernel job per service. The last handle closes only after the service
    /// is done, or when the launcher dies; descendants then terminate as well.
    pub struct Job {
        handle: usize,
    }

    impl Job {
        pub fn new() -> Result<Self, String> {
            let handle = unsafe { CreateJobObjectW(null(), null()) };
            if handle.is_null() {
                return Err(format!(
                    "无法创建进程 Job Object：{}",
                    io::Error::last_os_error()
                ));
            }
            let job = Self {
                handle: handle as usize,
            };
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(format!(
                    "无法启用子进程清理：{}",
                    io::Error::last_os_error()
                ));
            }
            Ok(job)
        }

        pub fn raw_handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
            self.handle as _
        }

        pub fn active_count(&self) -> Result<u32, String> {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
            if unsafe {
                QueryInformationJobObject(
                    self.handle as _,
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                Err(format!(
                    "读取进程组状态失败：{}",
                    io::Error::last_os_error()
                ))
            } else {
                Ok(info.ActiveProcesses)
            }
        }

        pub fn terminate(&self) -> Result<(), String> {
            if unsafe { TerminateJobObject(self.handle as _, 1) } == 0 {
                Err(format!("无法停止进程组：{}", io::Error::last_os_error()))
            } else {
                Ok(())
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle as _);
            }
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::{
        io,
        sync::atomic::{AtomicI32, Ordering},
    };
    pub struct Job {
        pid: AtomicI32,
    }
    impl Job {
        pub fn new() -> Result<Self, String> {
            Ok(Self {
                pid: AtomicI32::new(0),
            })
        }
        pub fn assign_pid(&self, pid: u32) {
            self.pid.store(pid as i32, Ordering::Release);
        }
        pub fn active_count(&self) -> Result<u32, String> {
            let pid = self.pid.load(Ordering::Acquire);
            Ok(u32::from(pid > 0 && unsafe { libc::kill(-pid, 0) } == 0))
        }
        pub fn terminate(&self) -> Result<(), String> {
            let pid = self.pid.load(Ordering::Acquire);
            if pid <= 0 {
                return Ok(());
            }
            if unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
                Ok(())
            } else {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    Ok(())
                } else {
                    Err(error.to_string())
                }
            }
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            let _ = self.terminate();
        }
    }
}

pub use platform::Job;
