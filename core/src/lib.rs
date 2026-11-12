//! Drainlog core: why did the battery drain overnight? Honest numbers only: every part of a
//! night's drop is labelled measured or estimated, and an unexplained remainder is always kept.

pub mod battery;
pub mod causes;
pub mod ledger;
pub mod parse;
pub mod report;
pub mod store;
