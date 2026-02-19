use anyhow::Result;

pub struct Journal;

impl Journal {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub async fn get_logs(_unit: Option<&str>, _lines: usize) -> Result<Vec<LogEntry>> {
        // TODO: Implement via libsystemd
        Ok(vec![])
    }
}

pub struct LogEntry {
    pub timestamp: String,
    pub unit: String,
    pub message: String,
    pub priority: u8,
}
