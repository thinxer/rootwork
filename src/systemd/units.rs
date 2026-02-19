use anyhow::Result;

pub struct UnitManager;

impl UnitManager {
    pub fn new() -> Self {
        Self
    }

    pub async fn start(&self,
        _unit: &str,
    ) -> Result<()> {
        // TODO: Implement via zbus
        Ok(())
    }

    pub async fn stop(&self,
        _unit: &str,
    ) -> Result<()> {
        // TODO: Implement via zbus
        Ok(())
    }

    pub async fn restart(&self,
        _unit: &str,
    ) -> Result<()> {
        // TODO: Implement via zbus
        Ok(())
    }

    pub async fn enable(&self,
        _unit: &str,
    ) -> Result<()> {
        // TODO: Implement via zbus
        Ok(())
    }

    pub async fn disable(&self,
        _unit: &str,
    ) -> Result<()> {
        // TODO: Implement via zbus
        Ok(())
    }
}
