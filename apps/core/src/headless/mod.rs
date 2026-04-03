mod apply;
mod discovery;
mod token;
mod validate;

#[cfg(test)]
mod test_helpers;

pub(crate) use apply::{apply_headless_configuration, apply_headless_settings};
pub(crate) use validate::validate_headless_configuration;

#[cfg(test)]
pub(crate) use test_helpers::{list_profile_source_ids, list_sources};

#[derive(Debug, Clone, Default)]
pub(crate) struct HeadlessApplyReport {
    pub(crate) installed_plugins: usize,
    pub(crate) created_sources: usize,
    pub(crate) updated_sources: usize,
    pub(crate) created_profiles: usize,
    pub(crate) updated_profiles: usize,
}
