pub mod units;
pub mod network;
pub mod dns;
pub mod host;
pub mod boot;
pub mod logs;

use ratatui::{
    layout::Rect,
    Frame,
};
use crossterm::event::KeyEvent;

/// Trait for all context views
pub trait Context {
    fn name(&self) -> &'static str;
    fn draw(&self, f: &mut Frame, area: Rect);
    fn handle_key(&mut self, key: KeyEvent);
    async fn tick(&mut self);
}
