//! Native connection strategy for the shared Postgres adapter: a deadpool-postgres pool over TCP.

use async_trait::async_trait;
use deadpool_postgres::{Manager, ManagerConfig, Object, Pool, RecyclingMethod};
use thrust_adapters_postgres::{ClientHandle, Connector};
use thrust_ports::DbError;
use tokio_postgres::{Client, NoTls};

pub struct PoolConnector {
    pool: Pool,
}

impl PoolConnector {
    /// `url` carries the password: callers read it from the environment, never from argv/files.
    pub fn from_url(url: &str, max_size: usize) -> Result<Self, String> {
        let cfg: tokio_postgres::Config = url.parse().map_err(|e| format!("bad database url: {e}"))?;
        let mgr = Manager::from_config(cfg, NoTls, ManagerConfig { recycling_method: RecyclingMethod::Fast });
        let pool = Pool::builder(mgr).max_size(max_size).build().map_err(|e| e.to_string())?;
        Ok(Self { pool })
    }

    pub async fn raw(&self) -> Result<Object, String> {
        self.pool.get().await.map_err(|e| e.to_string())
    }
}

struct Pooled(Object);

impl ClientHandle for Pooled {
    fn client(&self) -> &Client {
        &self.0
    }

    fn discard(self: Box<Self>) {
        // Remove the connection from the pool instead of recycling it: closing it makes the
        // server roll back whatever transaction was left open.
        let _ = Object::take(self.0);
    }
}

#[async_trait]
impl Connector for PoolConnector {
    async fn checkout(&self, _exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError> {
        let obj = self.pool.get().await.map_err(|e| DbError::Backend(format!("pool: {e}")))?;
        Ok(Box::new(Pooled(obj)))
    }
}
