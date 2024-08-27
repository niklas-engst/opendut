use std::sync::Mutex;

use crate::persistence::database::ConnectError;
use crate::persistence::error::PersistenceResult;
use crate::persistence::model::Persistable;
use crate::persistence::{Db, Storage};
use crate::resources::storage::volatile::VolatileResourcesStorage;
use crate::resources::storage::{DatabaseConnectInfo, Resource, ResourcesStorageApi};

pub struct PersistentResourcesStorage {
    storage: Storage,
}
impl PersistentResourcesStorage {
    pub async fn connect(database_connect_info: &DatabaseConnectInfo) -> Result<Self, ConnectError> {
        let db = Db {
            inner: Mutex::new(
                crate::persistence::database::connect(database_connect_info).await?
            )
        };
        let memory = VolatileResourcesStorage::default();
        let storage = Storage { db, memory };
        Ok(Self { storage })
    }
}
impl ResourcesStorageApi for PersistentResourcesStorage {
    fn insert<R>(&mut self, id: R::Id, resource: R) -> PersistenceResult<()>
    where R: Resource + Persistable {
        resource.insert(id, &mut self.storage)
    }

    fn remove<R>(&mut self, id: R::Id) -> PersistenceResult<Option<R>>
    where R: Resource + Persistable {
        R::remove(id, &mut self.storage)
    }

    fn get<R>(&self, id: R::Id) -> PersistenceResult<Option<R>>
    where R: Resource + Persistable + Clone {
        R::get(id, &self.storage)
    }

    fn list<R>(&self) -> PersistenceResult<Vec<R>>
    where R: Resource + Persistable + Clone {
        R::list(&self.storage)
    }
}
