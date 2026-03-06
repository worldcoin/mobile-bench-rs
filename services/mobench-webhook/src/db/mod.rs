pub mod deliveries;
pub mod dispatches;
pub mod models;
pub mod results;
pub mod runs;

use sqlx::PgPool;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct Repositories {
    pub deliveries: deliveries::DeliveryRepository,
    pub dispatches: dispatches::DispatchRepository,
    pub runs: runs::RunRepository,
    pub results: results::ResultRepository,
}

impl Repositories {
    pub fn new(pool: PgPool) -> Self {
        Self {
            deliveries: deliveries::DeliveryRepository::new(pool.clone()),
            dispatches: dispatches::DispatchRepository::new(pool.clone()),
            runs: runs::RunRepository::new(pool.clone()),
            results: results::ResultRepository::new(pool),
        }
    }
}
