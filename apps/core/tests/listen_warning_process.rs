use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[tokio::test]
async fn run_with_non_loopback_host_emits_warning_to_stderr() {
    let temp_root = create_temp_dir("listen-warning-process");
    let data_dir = temp_root.join("data");
    std::fs::create_dir_all(&data_dir).expect("创建 data 目录失败");

    let listen_port = reserve_available_port().await;
    let core_bin = core_binary_path();
    let child = Command::new(core_bin)
        .arg("run")
        .arg("--host")
        .arg("0.0.0.0")
        .arg("--port")
        .arg(listen_port.to_string())
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--secrets-backend")
        .arg("memory")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 subforge-core 失败");
    let mut child = ChildGuard::new(child);

    let api_base = format!("http://127.0.0.1:{listen_port}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("创建 HTTP 客户端失败");
    wait_until_healthy(&client, &api_base).await;

    let admin_token = std::fs::read_to_string(data_dir.join("admin_token"))
        .expect("读取 admin_token 文件失败")
        .trim()
        .to_string();
    assert!(!admin_token.is_empty(), "admin_token 不应为空");

    let shutdown_response = client
        .post(format!("{api_base}/api/system/shutdown"))
        .bearer_auth(admin_token)
        .send()
        .await
        .expect("调用 shutdown 失败");
    assert_eq!(shutdown_response.status(), reqwest::StatusCode::OK);

    let exit_status = wait_for_exit(child.inner_mut(), Duration::from_secs(10))
        .await
        .expect("读取 Core 退出状态失败");
    assert!(exit_status.success(), "Core 退出状态应为成功");

    let mut stderr_output = String::new();
    if let Some(mut stderr) = child.take_stderr() {
        stderr
            .read_to_string(&mut stderr_output)
            .expect("读取 stderr 失败");
    }
    assert!(
        stderr_output.contains("WARNING: 当前监听地址为 0.0.0.0"),
        "应输出非回环监听告警，stderr={stderr_output}"
    );

    child.disarm();
    let _ = std::fs::remove_dir_all(&temp_root);
}

async fn wait_until_healthy(client: &reqwest::Client, api_base: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let ready = client
            .get(format!("{api_base}/health"))
            .send()
            .await
            .map(|response| response.status() == reqwest::StatusCode::OK)
            .unwrap_or(false);
        if ready {
            return;
        }
        assert!(Instant::now() < deadline, "Core 启动超时，/health 未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_exit(
    child: &mut Child,
    timeout: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "等待子进程退出超时",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn reserve_available_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("分配可用端口失败");
    listener.local_addr().expect("读取可用端口失败").port()
}

fn core_binary_path() -> PathBuf {
    for key in ["CARGO_BIN_EXE_subforge-core", "CARGO_BIN_EXE_subforge_core"] {
        if let Ok(path) = std::env::var(key) {
            return PathBuf::from(path);
        }
    }

    let current_exe = std::env::current_exe().expect("读取当前测试进程路径失败");
    let debug_dir = current_exe
        .parent()
        .and_then(std::path::Path::parent)
        .expect("推断 target/debug 路径失败");
    let mut candidate = debug_dir.join("subforge-core");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    assert!(
        candidate.exists(),
        "未找到 subforge-core 可执行文件，候选路径: {}",
        candidate.display()
    );
    candidate
}

fn create_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "subforge-{prefix}-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录失败");
    dir
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn inner_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("子进程句柄不存在")
    }

    fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.as_mut().and_then(|child| child.stderr.take())
    }

    fn disarm(&mut self) {
        let _ = self.child.take();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
