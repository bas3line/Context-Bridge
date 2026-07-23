use sqlx::{Pool, Sqlite};

use crate::StorageError;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub(crate) async fn run(pool: &Pool<Sqlite>) -> Result<(), StorageError> {
    MIGRATOR.run(pool).await.map_err(StorageError::Migration)
}
