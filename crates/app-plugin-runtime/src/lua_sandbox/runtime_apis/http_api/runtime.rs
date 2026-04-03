use std::thread;
use std::time::Duration;

use tokio::runtime::Builder as TokioRuntimeBuilder;

pub(super) fn run_reqwest_blocking<T, F>(future: F) -> Result<T, String>
where
    T: Send + 'static,
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
{
    let handle = thread::spawn(move || {
        let runtime = TokioRuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("初始化异步运行时失败：{error}"))?;
        runtime.block_on(future)
    });

    handle
        .join()
        .map_err(|_| "HTTP 请求线程异常退出".to_string())?
}

pub(super) fn retry_backoff(base_delay: Duration, retry_attempt: usize) -> Duration {
    let base_delay = if base_delay.is_zero() {
        Duration::from_millis(100)
    } else {
        base_delay
    };
    let shift = retry_attempt.saturating_sub(1).min(8);
    base_delay.saturating_mul(1_u32 << shift)
}
