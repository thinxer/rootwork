use ratatui::{
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

pub struct Help {
    pub visible: bool,
}

impl Help {
    pub fn new() -> Self {
        Self { visible: false }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

impl Widget for Help {
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        if !self.visible {
            return;
        }

        let block = Block::default()
            .title(" Help ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::palette::yellow()))
            .style(Style::default().bg(crate::palette::black()));

        let help_text = r#"Rootwork - systemd TUI

Global:
    q, Q          Quit
    ?             Toggle help
    Tab           Next context
    Shift+Tab     Previous context
    1-6           Jump to context

Navigation:
    j, ↓          Down
    k, ↑          Up
    g             Top of list
    G             Bottom of list
    /             Search/filter
    Esc           Clear filter"#;

        let paragraph = Paragraph::new(help_text).block(block);
        Clear.render(area, buf);
        paragraph.render(area, buf);
    }
}
