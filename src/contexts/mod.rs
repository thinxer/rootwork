pub mod boot;
pub mod dns;
pub mod host;
pub mod logs;
pub mod network;
pub mod units;

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

/// Trait for all context views
pub trait Context {
    fn name(&self) -> &'static str;
    fn draw(&self, f: &mut Frame, area: Rect);
    fn handle_key(&mut self, key: KeyEvent);
    async fn tick(&mut self);
}
