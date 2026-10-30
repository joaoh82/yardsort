//! A real PostgreSQL for a test, when there is one: `YARDSORT_TEST_DATABASE_URL`, which
//! `just server-test` sets to the development environment's `yardsort_test` database and CI to
//! its own service. Each test gets a schema of its own, dropped when it passes.
//!
//! Without the variable the PostgreSQL half of a test is skipped, and says so — unless
//! `YARDSORT_REQUIRE_TEST_DATABASE` is set, as CI sets it on Linux, where a missing database is
//! then a failure rather than a quiet pass. (CI's macOS and Windows runners have no containers.)

use std::sync::Arc;

use yardsort_server::postgres::PgRecords;

pub const URL_VAR: &str = "YARDSORT_TEST_DATABASE_URL";
pub const REQUIRE_VAR: &str = "YARDSORT_REQUIRE_TEST_DATABASE";

pub struct TestDatabase {
    pub records: Arc<PgRecords>,
    pub config: tokio_postgres::Config,
    url: String,
    schema: String,
}

/// A fresh schema, migrated, or `None` when no database is configured.
pub async fn database() -> Option<TestDatabase> {
    let empty = empty().await?;
    let records = PgRecords::connect_with(empty.config.clone())
        .await
        .expect("connecting and migrating");
    Some(TestDatabase {
        records: Arc::new(records),
        config: empty.config,
        url: empty.url,
        schema: empty.schema,
    })
}

/// A fresh schema with nothing in it, not even the server's tables.
pub async fn empty() -> Option<EmptySchema> {
    let Ok(url) = std::env::var(URL_VAR) else {
        assert!(
            std::env::var_os(REQUIRE_VAR).is_none(),
            "{REQUIRE_VAR} is set, so {URL_VAR} must be too"
        );
        eprintln!("skipped: {URL_VAR} is not set (`just server-test` sets it)");
        return None;
    };
    let schema = format!("test_{}", uuid::Uuid::new_v4().simple());
    admin(&url, &format!("CREATE SCHEMA {schema}")).await;
    let mut config: tokio_postgres::Config = url.parse().expect("a PostgreSQL URL");
    config.options(format!("-c search_path={schema}"));
    Some(EmptySchema {
        config,
        url,
        schema,
    })
}

pub struct EmptySchema {
    pub config: tokio_postgres::Config,
    url: String,
    schema: String,
}

impl TestDatabase {
    /// A URL for this test's schema, as a process outside the test would be given it.
    pub fn url(&self) -> String {
        let separator = if self.url.contains('?') { '&' } else { '?' };
        format!(
            "{}{separator}options=-c%20search_path%3D{}",
            self.url, self.schema
        )
    }

    pub async fn drop(self) {
        admin(&self.url, &format!("DROP SCHEMA {} CASCADE", self.schema)).await;
    }
}

impl EmptySchema {
    pub async fn drop(self) {
        admin(&self.url, &format!("DROP SCHEMA {} CASCADE", self.schema)).await;
    }
}

async fn admin(url: &str, sql: &str) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("connecting to the test database");
    tokio::spawn(connection);
    client.batch_execute(sql).await.expect(sql);
}
