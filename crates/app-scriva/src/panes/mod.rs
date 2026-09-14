//! The panes beside the page: Review on the right, and — from phase 5 of the
//! redesign — Navigate on the left.
//!
//! A pane is a panel of the document surface, not of the window, so the
//! toolbar and the status bar run the full width and the page is what is
//! left. Each pane reads its rows from the model before it draws, because a
//! closure drawing a panel cannot borrow the application while the
//! application is drawing it, and returns a [`crate::app::Command`] rather
//! than doing anything itself.

pub(crate) mod navigate;
pub(crate) mod review;
