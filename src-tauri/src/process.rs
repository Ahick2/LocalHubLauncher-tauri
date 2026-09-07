use crate::{
    decoder::LineDecoder,
    job::Job,
    launch::build_spec,
    logs::{LogStore, LogStream},
    model::{now_ms, LaunchItem, ProcessState, RuntimeStatus},
    native_process::{NativeChild, OutputPipe},
};
use parking_lot::{Mutex, RwLock};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    sync::{watch, Mutex as AsyncMutex},
};
use uuid::Uuid;

type ChangeHandler = Arc<dyn Fn() + Send + Sync>;

struct Instance {
    generation: u64,
    job: Arc<Job>,
    requested_stop: AtomicBool,
    completed: watch::Sender<bool>,
}

pub struct ProcessManager {
    directory: PathBuf,
    logs: Arc<LogStore>,
    instances: Mutex<HashMap<Uuid, Arc<Instance>>>,
    statuses: Mutex<HashMap<Uuid, RuntimeStatus>>,
    gates: Mutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>,
    generation: AtomicU64,
    changed: RwLock<Option<ChangeHandler>>,
}

impl ProcessManager {
    pub fn new(directory: PathBuf, logs: Arc<LogStore>) -> Arc<Self> {
        Arc::new(Self {
            directory,
            logs,
            instances: Mutex::new(HashMap::new()),
            statuses: Mutex::new(HashMap::new()),
            gates: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            changed: RwLock::new(None),
        })
    }

    pub fn set_change_handler(&self, handler: ChangeHandler) {
        *self.changed.write() = Some(handler);
    }

    fn notify(&self) {
        let handler = self.changed.read().clone();
        if let Some(handler) = handler {
            handler();
        }
    }

    fn gate(&self, id: Uuid) -> Arc<AsyncMutex<()>> {
        self.gates
            .lock()
            .entry(id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    pub fn statuses(&self) -> Vec<RuntimeStatus> {
        self.statuses.lock().values().cloned().collect()
    }

    pub fn is_running(&self, id: Uuid) -> bool {
        self.instances.lock().contains_key(&id)
    }

    pub fn active_ids(&self) -> Vec<Uuid> {
        self.instances.lock().keys().copied().collect()
    }

    pub async fn start(self: &Arc<Self>, item: &LaunchItem) -> Result<(), String> {
        let gate = self.gate(item.id);
        let _guard = gate.lock().await;
        self.start_unlocked(item)
    }

    pub async fn stop(self: &Arc<Self>, id: Uuid) -> Result<(), String> {
        let gate = self.gate(id);
        let _guard = gate.lock().await;
        self.stop_unlocked(id).await
    }

    pub async fn restart(self: &Arc<Self>, item: &LaunchItem) -> Result<(), String> {
        let gate = self.gate(item.id);
        let _guard = gate.lock().await;
        self.stop_unlocked(item.id).await?;
        self.start_unlocked(item)
    }

    fn start_unlocked(self: &Arc<Self>, item: &LaunchItem) -> Result<(), String> {
        if self.is_running(item.id) {
            return Ok(());
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut status = RuntimeStatus::stopped(item.id);
        status.generation = generation;
        status.state = ProcessState::Starting;
        self.statuses.lock().insert(item.id, status);
        self.logs.ensure(item.id, &item.name);
        self.notify();
        let result = (|| {
            let spec = build_spec(item, &self.directory)?;
            let job = Arc::new(Job::new()?);
            let child = NativeChild::spawn(&spec, &job)?;
            Ok::<_, String>((job, child))
        })();
        let (job, mut child) = match result {
            Ok(result) => result,
            Err(error) => {
                self.update(item.id, generation, |status| {
                    status.state = ProcessState::Stopped;
                    status.error = Some(error.clone());
                });
                self.logs.append(
                    item.id,
                    &item.name,
                    format!("启动失败：{error}"),
                    LogStream::Stderr,
                );
                return Err(error);
            }
        };
        let (completed, _) = watch::channel(false);
        let instance = Arc::new(Instance {
            generation,
            job,
            requested_stop: AtomicBool::new(false),
            completed,
        });
        self.instances.lock().insert(item.id, instance.clone());
        self.update(item.id, generation, |status| {
            status.state = ProcessState::Running;
            status.pid = Some(child.pid);
            status.started_at = Some(now_ms());
        });
        self.logs.append(
            item.id,
            &item.name,
            format!("已启动，PID {}", child.pid),
            LogStream::System,
        );
        if !item.hide_window {
            self.logs.append(
                item.id,
                &item.name,
                "此服务在独立终端显示输出；启用“后台运行”可在此捕获日志。",
                LogStream::System,
            );
        }
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            readers.push(tokio::spawn(read_output(
                stdout,
                item.clone(),
                self.logs.clone(),
                LogStream::Stdout,
            )));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.push(tokio::spawn(read_output(
                stderr,
                item.clone(),
                self.logs.clone(),
                LogStream::Stderr,
            )));
        }
        let manager = self.clone();
        let item = item.clone();
        tokio::spawn(async move {
            manager.monitor(item, instance, child, readers).await;
        });
        Ok(())
    }

    async fn monitor(
        self: Arc<Self>,
        item: LaunchItem,
        instance: Arc<Instance>,
        mut child: NativeChild,
        readers: Vec<tokio::task::JoinHandle<()>>,
    ) {
        let mut exit_code = None;
        let mut query_error_reported = false;
        loop {
            if exit_code.is_none() {
                match child.try_wait() {
                    Ok(Some(code)) => {
                        exit_code = Some(code);
                        // The wrapper may exit while the actual service keeps
                        // running. Its entire job, not just this PID, is tracked.
                        self.update(item.id, instance.generation, |status| {
                            status.pid = None;
                        });
                    }
                    Ok(None) => {}
                    Err(error) if !query_error_reported => {
                        self.logs.append(
                            item.id,
                            &item.name,
                            format!("读取进程状态失败：{error}"),
                            LogStream::Stderr,
                        );
                        query_error_reported = true;
                    }
                    Err(_) => {}
                }
            }
            match instance.job.active_count() {
                Ok(0) if exit_code.is_some() => break,
                Err(error) if !query_error_reported => {
                    self.logs
                        .append(item.id, &item.name, error, LogStream::Stderr);
                    query_error_reported = true;
                }
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        // Do not dispose the streams on Exited: drain buffered output first.
        for mut reader in readers {
            if tokio::time::timeout(Duration::from_secs(2), &mut reader)
                .await
                .is_err()
            {
                reader.abort();
                self.logs.append(
                    item.id,
                    &item.name,
                    "日志管道未及时关闭，已结束读取。",
                    LogStream::Stderr,
                );
            }
        }
        let requested_stop = instance.requested_stop.load(Ordering::Acquire);
        let failed = !requested_stop && exit_code != Some(0);
        let message = if requested_stop {
            "已停止".to_owned()
        } else {
            format!("进程组已退出（代码 {}）", exit_code.unwrap_or(-1))
        };
        self.logs.append(
            item.id,
            &item.name,
            message,
            if failed {
                LogStream::Stderr
            } else {
                LogStream::System
            },
        );
        {
            let mut instances = self.instances.lock();
            if instances
                .get(&item.id)
                .is_some_and(|current| Arc::ptr_eq(current, &instance))
            {
                instances.remove(&item.id);
            }
        }
        self.update(item.id, instance.generation, |status| {
            status.state = ProcessState::Stopped;
            status.pid = None;
            status.exit_code = exit_code;
            status.requested_stop = requested_stop;
            status.error =
                failed.then(|| format!("进程异常退出，代码 {}", exit_code.unwrap_or(-1)));
        });
        instance.completed.send_replace(true);
    }

    async fn stop_unlocked(&self, id: Uuid) -> Result<(), String> {
        let instance = self.instances.lock().get(&id).cloned();
        let Some(instance) = instance else {
            return Ok(());
        };
        instance.requested_stop.store(true, Ordering::Release);
        self.update(id, instance.generation, |status| {
            status.state = ProcessState::Stopping;
            status.error = None;
        });
        if let Err(error) = instance.job.terminate() {
            // Keep readers and tracking alive so a failed stop is retryable.
            instance.requested_stop.store(false, Ordering::Release);
            self.update(id, instance.generation, |status| {
                status.state = ProcessState::Running;
                status.error = Some(error.clone());
            });
            return Err(error);
        }
        let mut completed = instance.completed.subscribe();
        if tokio::time::timeout(Duration::from_secs(10), completed.wait_for(|value| *value))
            .await
            .is_err()
        {
            if !self.is_running(id) {
                return Ok(());
            }
            instance.requested_stop.store(false, Ordering::Release);
            let error =
                "进程组在 10 秒内没有完全退出，已保留启动项和日志，可再次尝试停止".to_owned();
            self.update(id, instance.generation, |status| {
                status.state = ProcessState::Running;
                status.error = Some(error.clone());
            });
            return Err(error);
        }
        Ok(())
    }

    fn update(&self, id: Uuid, generation: u64, update: impl FnOnce(&mut RuntimeStatus)) {
        {
            let mut statuses = self.statuses.lock();
            let Some(status) = statuses.get_mut(&id) else {
                return;
            };
            if status.generation != generation {
                return;
            }
            update(status);
        }
        self.notify();
    }

    pub fn forget(&self, id: Uuid) -> Result<(), String> {
        if self.is_running(id) {
            return Err("服务仍在运行，不能移除其状态".into());
        }
        self.statuses.lock().remove(&id);
        self.logs.remove(id);
        Ok(())
    }

    pub fn kill_all(&self) {
        for instance in self.instances.lock().values() {
            let _ = instance.job.terminate();
        }
    }
}

async fn read_output(
    mut pipe: OutputPipe,
    item: LaunchItem,
    logs: Arc<LogStore>,
    stream: LogStream,
) {
    let mut decoder = LineDecoder::default();
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                for line in decoder.feed(&buffer[..count]) {
                    logs.append(item.id, &item.name, line, stream);
                }
            }
            Err(error) => {
                if !matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
                ) {
                    logs.append(
                        item.id,
                        &item.name,
                        format!("日志读取失败：{error}"),
                        LogStream::Stderr,
                    );
                }
                break;
            }
        }
    }
    for line in decoder.finish() {
        logs.append(item.id, &item.name, line, stream);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::model::LaunchType;

    fn manager() -> (tempfile::TempDir, Arc<ProcessManager>, Arc<LogStore>) {
        let dir = tempfile::tempdir().unwrap();
        let logs = Arc::new(LogStore::default());
        let manager = ProcessManager::new(dir.path().into(), logs.clone());
        (dir, manager, logs)
    }

    async fn wait_stopped(manager: &ProcessManager, id: Uuid) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while manager.is_running(id) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn short_lived_process_drains_the_last_output_line() {
        let (_dir, manager, logs) = manager();
        let mut item = LaunchItem::new("short", "for /L %i in (1,1,200) do @echo line-%i");
        item.launch_type = LaunchType::Command;
        manager.start(&item).await.unwrap();
        wait_stopped(&manager, item.id).await;
        let records = logs.read(Some(item.id));
        assert_eq!(
            records
                .iter()
                .filter(|record| record.text.starts_with("line-"))
                .count(),
            200
        );
        assert!(records.iter().any(|record| record.text == "line-200"));
    }

    #[tokio::test]
    async fn concurrent_starts_are_serialized_and_restart_changes_generation() {
        let (_dir, manager, _logs) = manager();
        let mut item = LaunchItem::new("sleep", "Start-Sleep -Seconds 30");
        item.launch_type = LaunchType::WindowsPowerShell;
        let (a, b) = tokio::join!(manager.start(&item), manager.start(&item));
        a.unwrap();
        b.unwrap();
        assert_eq!(manager.active_ids(), vec![item.id]);
        let generation = manager.statuses()[0].generation;
        manager.restart(&item).await.unwrap();
        assert!(manager.statuses()[0].generation > generation);
        assert!(manager.is_running(item.id));
        manager.stop(item.id).await.unwrap();
        assert!(!manager.is_running(item.id));
    }

    #[tokio::test]
    async fn a_child_remains_controllable_after_its_wrapper_exits() {
        let (_dir, manager, _logs) = manager();
        let mut item = LaunchItem::new(
            "wrapper",
            r#"start "" /b powershell.exe -NoProfile -Command "Start-Sleep -Seconds 30""#,
        );
        item.launch_type = LaunchType::Command;
        manager.start(&item).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while manager.statuses()[0].pid.is_some() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(manager.is_running(item.id));
        manager.stop(item.id).await.unwrap();
        assert!(!manager.is_running(item.id));
    }
}
