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

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "portfolios",
        sql: include_str!("../migrations/0002_portfolios.sql"),
    },
];

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

#[cfg(test)]
mod upgrade_tests {
    use super::testing::TestDb;
    use super::*;

    /// A schema at version 1 with one account, history and a global automation setting becomes
    /// one shared portfolio, "Main", that keeps all of it.
    #[tokio::test]
    async fn version_1_data_becomes_the_main_portfolio() {
        let Some(db) = TestDb::new(DataSource::Demo).await else {
            eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
            return;
        };
        // Rebuild the schema as version 1 left it.
        let schema = format!("{}_v1", db.schema);
        let client = db.pool.get().await.unwrap();
        client
            .batch_execute(&format!(
                "CREATE SCHEMA {schema};
                 SET search_path TO {schema};
                 CREATE TABLE {schema}.schema_migrations (
                     version INT PRIMARY KEY, name TEXT NOT NULL, checksum TEXT NOT NULL,
                     applied_at TIMESTAMPTZ NOT NULL DEFAULT now());"
            ))
            .await
            .unwrap();
        client.batch_execute(MIGRATIONS[0].sql).await.unwrap();
        client
            .execute(
                &format!("INSERT INTO {schema}.schema_migrations (version, name, checksum) VALUES (1, 'init', $1)"),
                &[&checksum(MIGRATIONS[0].sql)],
            )
            .await
            .unwrap();
        client
            .batch_execute(
                "INSERT INTO instance_meta (key, value) VALUES ('data_source', '{\"source\": \"demo\"}');
                 INSERT INTO settings (key, value) VALUES ('automation', '{\"mode\": \"approval\", \"note\": \"careful\"}');
                 INSERT INTO accounts (name, initial_cash, cash, inception_date, status) VALUES ('Paper', 1000, 1000, '2026-01-02', 'archived');
                 INSERT INTO accounts (name, initial_cash, cash, inception_date) VALUES ('Paper', 5000, 5000, '2026-02-02');
                 INSERT INTO skipped_slots (trade_date, slot, created_by) VALUES ('2026-10-06', 'close', 'admin');
                 INSERT INTO trading_restrictions (symbol, mode, starts_on, created_by) VALUES ('NVDA', 'lock', '2026-10-01', 'admin');
                 INSERT INTO users (username, password_hash, role) VALUES ('ops', 'x', 'viewer');",
            )
            .await
            .unwrap();
        drop(client);

        // Upgrade through a pool pinned to that schema.
        let url = std::env::var("HONE_QUANT_TEST_DATABASE_URL").unwrap();
        let mut pg = <tokio_postgres::Config as std::str::FromStr>::from_str(&url).unwrap();
        pg.options(format!("-c search_path={schema}"));
        let cfg = crate::config::DbConfig {
            pg,
            schema: schema.clone(),
            pool_size: 2,
            display: "test".into(),
            borrowed_from_honeclaw: false,
        };
        let pool = create_pool(&cfg).unwrap();
        assert_eq!(
            migrate(&pool, &schema, DataSource::Demo).await.unwrap(),
            vec![2]
        );
        let client = pool.get().await.unwrap();
        let row = client
            .query_one(
                "SELECT id, name, owner, automation->>'mode', automation->>'note', status FROM portfolios",
                &[],
            )
            .await
            .unwrap();
        let main: i64 = row.get(0);
        assert_eq!(row.get::<_, String>(1), "Main");
        assert_eq!(row.get::<_, Option<String>>(2), None);
        assert_eq!(row.get::<_, String>(3), "approval");
        assert_eq!(row.get::<_, String>(4), "careful");
        assert_eq!(row.get::<_, String>(5), "active");
        let unassigned: i64 = client
            .query_one(
                "SELECT count(*) FROM accounts WHERE portfolio_id IS DISTINCT FROM $1",
                &[&main],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(unassigned, 0, "both accounts belong to Main");
        let skipped: i64 = client
            .query_one("SELECT portfolio_id FROM skipped_slots", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(skipped, main);
        let scope: Option<i64> = client
            .query_one("SELECT portfolio_id FROM trading_restrictions", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            scope, None,
            "existing restrictions keep applying everywhere"
        );
        assert!(
            client
                .query_opt("SELECT 1 FROM settings WHERE key = 'automation'", &[])
                .await
                .unwrap()
                .is_none()
        );
        // One active account per portfolio, any number of portfolios; members exist.
        let second = client
            .execute(
                "INSERT INTO accounts (portfolio_id, name, initial_cash, cash, inception_date) VALUES ($1, 'Paper', 1, 1, '2026-10-05')",
                &[&main],
            )
            .await;
        assert!(second.is_err(), "Main already has an active account");
        client
            .batch_execute(
                "INSERT INTO portfolios (name, owner, created_by) VALUES ('Alice', 'alice', 'alice');
                 INSERT INTO accounts (portfolio_id, name, initial_cash, cash, inception_date)
                   SELECT id, 'Paper', 1, 1, '2026-10-05' FROM portfolios WHERE owner = 'alice';
                 INSERT INTO users (username, password_hash, role) VALUES ('alice', 'x', 'member');",
            )
            .await
            .unwrap();
        assert!(
            client
                .execute(
                    "INSERT INTO portfolios (name, owner, created_by) VALUES ('ALICE ', 'alice', 'alice')",
                    &[],
                )
                .await
                .is_err(),
            "names are unique per owner"
        );
        drop(client);
        pool.close();
        let client = db.pool.get().await.unwrap();
        client
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
        drop(client);
        db.drop().await;
    }
}
