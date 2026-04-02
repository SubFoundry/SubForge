//! app-transport：网络传输拟真档位与请求策略。

mod error;
mod profiles;
#[cfg(test)]
mod tests;

pub use error::{TransportError, TransportResult};
pub use profiles::{
    BrowserChromeProfile, NetworkProfileFactory, StandardProfile, TransportProfile,
};
