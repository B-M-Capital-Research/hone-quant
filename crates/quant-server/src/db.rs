//! PostgreSQL pool and migrations.
//!
//! - Every pooled connection has `search_path` pinned to the hone-quant schema (see `config`).
//! - Migrations are embedded SQL files applied in order inside one transaction under an advisory
//!   lock, and recorded with a checksum so an edited, already-applied migration is detected
//!   instead of silently drifting.
//! - The schema remembers which data source created it; a demo server cannot open a schema that
//!   holds real FMP data (and vice versa).

use anyhow::{Context, Result, bail};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use sha2::{Digest, Sha256};
use tokio_postgres::NoTls;

use crate::config::DbConfig;
use crate::market::DataSource;

pub type Client = deadpool_postgres::Object;

struct Migration {
    version: i32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "init",
    sql: include_str!("../migrations/0001_init.sql"),
}];

pub fn create_pool(cfg: &DbConfig) -> Result<Pool> {
    let manager = Manager::from_config(
        cfg.pg.clone(),
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    Pool::builder(manager)
        .max_size(cfg.pool_size)
        .runtime(deadpool_postgres::Runtime::Tokio1)
        .build()
        .context("cannot build PostgreSQL pool")
}

fn checksum(sql: &str) -> String {
    hex::encode(Sha256::digest(sql.as_bytes()))
}

/// Advisory-lock key derived from a namespaced string, matching honeclaw's convention of
/// `hashtextextended('<namespace>:...')` so keys from the two apps never collide.
pub fn lock_key(name: &str) -> String {
    format!("hone_quant:{name}")
}

/// Creates the schema if needed and applies pending migrations.
pub async fn migrate(pool: &Pool, schema: &str, source: DataSource) -> Result<Vec<i32>> {
    let mut client = pool.get().await.context("cannot connect to PostgreSQL")?;
    // The identifier was validated by `config::valid_schema_name`.
    client
        .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .await
        .with_context(|| {
            format!(
                "cannot create schema {schema}; create it once as a superuser: CREATE SCHEMA {schema} AUTHORIZATION <role>"
            )
        })?;
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&lock_key(&format!("migrate:{schema}"))],
    )
    .await?;
    tx.batch_execute(&format!(
        "CREATE TABLE IF NOT EXISTS {schema}.schema_migrations (
            version INT PRIMARY KEY,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"
    ))
    .await?;
    let rows = tx
        .query(
            &format!("SELECT version, checksum FROM {schema}.schema_migrations"),
            &[],
        )
        .await?;
    let applied: Vec<(i32, String)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
    let mut newly = Vec::new();
    for migration in MIGRATIONS {
        let sum = checksum(migration.sql);
        if let Some((_, existing)) = applied.iter().find(|(v, _)| *v == migration.version) {
            if *existing != sum {
                bail!(
                    "migration {} ({}) was modified after being applied; refusing to start",
                    migration.version,
                    migration.name
                );
            }
            continue;
        }
        tx.batch_execute(migration.sql).await.with_context(|| {
            format!(
                "migration {} ({}) failed",
                migration.version, migration.name
            )
        })?;
        tx.execute(
            &format!("INSERT INTO {schema}.schema_migrations (version, name, checksum) VALUES ($1, $2, $3)"),
            &[&migration.version, &migration.name, &sum],
        )
        .await?;
        newly.push(migration.version);
    }

    // Data-source guard.
    let existing = tx
        .query_opt(
            "SELECT value->>'source' FROM instance_meta WHERE key = 'data_source'",
            &[],
        )
        .await?
        .and_then(|row| row.get::<_, Option<String>>(0));
    match existing {
        None => {
            tx.execute(
                "INSERT INTO instance_meta (key, value) VALUES ('data_source', jsonb_build_object('source', $1::text, 'initialized_at', now()))",
                &[&source.as_str()],
            )
            .await?;
        }
        Some(existing) if existing != source.as_str() => {
            bail!(
                "schema {schema} holds {existing} data but the server is configured for {}; use a different HONE_QUANT_DB_SCHEMA",
                source.as_str()
            );
        }
        Some(_) => {}
    }
    tx.commit().await?;
    Ok(newly)
}

pub async fn health(pool: &Pool) -> bool {
    match tokio::time::timeout(std::time::Duration::from_secs(5), pool.get()).await {
        Ok(Ok(client)) => client.query_one("SELECT 1", &[]).await.is_ok(),
        _ => false,
    }
}

#[cfg(test)]
pub mod testing {
    //! Integration-test helpers: each test gets a throw-away schema in the database named by
    //! `HONE_QUANT_TEST_DATABASE_URL`; tests are skipped when it is unset.
    use super::*;
    use std::str::FromStr;

    pub struct TestDb {
        pub pool: Pool,
        pub schema: String,
        admin: tokio_postgres::Config,
    }

    impl TestDb {
        pub async fn new(source: DataSource) -> Option<Self> {
            let url = std::env::var("HONE_QUANT_TEST_DATABASE_URL").ok()?;
            let schema = format!("hq_test_{}", uuid::Uuid::new_v4().simple());
            let admin = tokio_postgres::Config::from_str(&url).expect("valid test URL");
            let mut pg = admin.clone();
            pg.options(format!("-c search_path={schema}"));
            let cfg = DbConfig {
                pg,
                schema: schema.clone(),
                pool_size: 4,
                display: "test".into(),
                borrowed_from_honeclaw: false,
            };
            let pool = create_pool(&cfg).expect("pool");
            migrate(&pool, &schema, source)
                .await
                .expect("migrations apply");
            Some(Self {
                pool,
                schema,
                admin,
            })
        }

        pub async fn drop(self) {
            let schema = self.schema.clone();
            self.pool.close();
            if let Ok((client, conn)) = self.admin.connect(NoTls).await {
                tokio::spawn(conn);
                let _ = client
                    .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TestDb;
    use super::*;

    #[tokio::test]
    async fn migrations_are_idempotent_and_guard_the_data_source() {
        let Some(db) = TestDb::new(DataSource::Demo).await else {
            eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
            return;
        };
        // Re-running applies nothing new.
        let again = migrate(&db.pool, &db.schema, DataSource::Demo)
            .await
            .unwrap();
        assert!(again.is_empty());
        // Opening the demo schema as an FMP server is refused.
        let err = migrate(&db.pool, &db.schema, DataSource::Fmp)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("holds demo data"), "{err}");
        // Every table landed in the test schema, none in public.
        let client = db.pool.get().await.unwrap();
        let count: i64 = client
            .query_one(
                "SELECT count(*) FROM information_schema.tables WHERE table_schema = $1",
                &[&db.schema],
            )
            .await
            .unwrap()
            .get(0);
        assert!(
            count >= 25,
            "expected the full schema, found {count} tables"
        );
        drop(client);
        db.drop().await;
    }
}
