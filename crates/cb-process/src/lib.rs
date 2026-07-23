//! Interactive child-process launching without terminal-stream transcript scraping.

mod child;
mod command;
mod pty;
mod signals;
mod terminal;

pub use child::*;
pub use command::*;
pub use terminal::*;

use signals::{install_forwarded_signals, wait_with_forwarded_signals};
