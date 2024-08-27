use url::Url;

use crate::persistence::database::ConnectError;
use crate::persistence::error::PersistenceResult;
use crate::persistence::resources::Persistable;
use crate::resources::storage::persistent::PersistentResourcesStorage;
use crate::resources::storage::volatile::VolatileResourcesStorage;
use crate::resources::Resource;

pub mod volatile;
pub mod persistent;

#[cfg(test)]
mod tests;

pub enum ResourcesStorage {
    Persistent(PersistentResourcesStorage),
    Volatile(VolatileResourcesStorage),
}
impl ResourcesStorage {
    pub async fn connect(options: PersistenceOptions) -> Result<Self, ConnectionError> {
        let storage = match options {
            PersistenceOptions::Enabled { database_connect_info } => {
                let storage = PersistentResourcesStorage::connect(&database_connect_info).await
                    .map_err(|cause| ConnectionError::Database { url: database_connect_info.url, source: cause })?;
                ResourcesStorage::Persistent(storage)
            }
            PersistenceOptions::Disabled => {
                ResourcesStorage::Volatile(VolatileResourcesStorage::default())
            }
        };
        Ok(storage)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("Failed to connect to database at '{url}'")]
    Database { url: Url, #[source] source: ConnectError },
}

pub enum PersistenceOptions {
    Enabled { database_connect_info: DatabaseConnectInfo },
    Disabled,
}
impl PersistenceOptions {
    pub fn load(config: &config::Config) -> Result<Self, opendut_util::settings::LoadError> {
        use opendut_util::settings::LoadError;

        let persistence_enabled = config.get_bool("persistence.enabled")?;

        if persistence_enabled {
            let url = {
                let field = "persistence.database.url";
                let value = config.get_string(field)
                    .map_err(|source| LoadError::FieldRetrieval { field, source: Box::new(source) })?;

                Url::parse(&value)
                    .map_err(|cause| LoadError::Parse {
                        field: field.to_owned(),
                        value,
                        source: Box::new(cause)
                    })?
            };

            let username = {
                let field = "persistence.database.username";
                config.get_string(field)
                    .map_err(|source| LoadError::FieldRetrieval { field, source: Box::new(source) })?
            };

            let password = {
                let field = "persistence.database.password";
                let value = config.get_string(field)
                    .map_err(|source| LoadError::FieldRetrieval { field, source: Box::new(source) })?;
                Password { secret: value }
            };

            Ok(PersistenceOptions::Enabled {
                database_connect_info: DatabaseConnectInfo {
                    url,
                    username,
                    password,
                }
            })
        } else {
            Ok(PersistenceOptions::Disabled)
        }
    }
}
#[derive(Clone)]
pub struct DatabaseConnectInfo {
    pub url: Url,
    pub username: String,
    pub password: Password,
}
///Wrapper for String without Debug and Display
#[derive(Clone)]
pub struct Password { secret: String }
impl Password {
    pub fn secret(&self) -> &str {
        &self.secret
    }

    #[cfg(test)]
    pub fn new_static(secret: &'static str) -> Self {
        Self { secret: secret.to_owned() }
    }
}

pub trait ResourcesStorageApi {
    fn insert<R>(&mut self, id: R::Id, resource: R) -> PersistenceResult<()>
    where R: Resource + Persistable;

    fn remove<R>(&mut self, id: R::Id) -> PersistenceResult<Option<R>>
    where R: Resource + Persistable;

    fn get<R>(&self, id: R::Id) -> PersistenceResult<Option<R>>
    where R: Resource + Persistable + Clone;

    fn list<R>(&self) -> PersistenceResult<Vec<R>>
    where R: Resource + Persistable + Clone;
}
