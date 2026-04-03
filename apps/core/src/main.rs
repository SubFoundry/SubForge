mod bootstrap;
mod cli;
mod config;
mod headless;
mod security;
mod settings_seed;
#[cfg(test)]
mod tests;

#[tokio::main]
async fn main() {
    if let Err(err) = bootstrap::run_cli().await {
        eprintln!("错误: {err:#}");
        std::process::exit(1);
    }
}
