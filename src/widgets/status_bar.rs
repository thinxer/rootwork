use ratatui::{
    style::Style,
    widgets::{Block, Borders, Widget},
};

pub struct StatusBar;

impl StatusBar {
    pub fn new() -> Self {
        Self
    }
}

impl Widget for StatusBar {
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(crate::palette::gray()));
        block.render(area, buf);
    }
}
