use ratatui::{
    style::Style,
    widgets::{Block, Borders, Widget},
};

pub struct LogView;

impl LogView {
    pub fn new() -> Self {
        Self
    }
}

impl Widget for LogView {
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        let block = Block::default()
            .title(" Logs ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::palette::white()));
        block.render(area, buf);
    }
}
