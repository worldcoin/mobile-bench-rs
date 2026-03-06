use anyhow::Result;

use crate::{AppState, db::models::DeliveryRecord};

pub async fn handle_delivery(_state: &AppState, _delivery: &DeliveryRecord) -> Result<()> {
    Ok(())
}
