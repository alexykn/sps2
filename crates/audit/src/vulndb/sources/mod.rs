//! Data source modules for vulnerability database updates

mod github;
mod nvd;
mod osv;

pub(crate) use github::update_from_github;
pub(crate) use nvd::update_from_nvd;
pub(crate) use osv::update_from_osv;
