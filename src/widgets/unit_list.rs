use ratatui::{
    style::Style,
    widgets::{Block, Borders, Widget},
};

pub struct UnitList;

impl UnitList {
    pub fn new() -> Self {
        Self
    }
}

impl Widget for UnitList {
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        let block = Block::default()
            .title(" Units ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::palette::white()));
        block.render(area, buf);
    }
}
