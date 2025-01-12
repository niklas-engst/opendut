use crate::persistence::error::PersistenceResult;
use crate::persistence::resources::Persistable;
use crate::resources::resource::Resource;
use crate::resources::storage::persistent::PersistentResourcesTransaction;
use crate::resources::storage::ResourcesStorageApi;
use crate::resources::storage::volatile::VolatileResourcesTransaction;
use crate::resources::subscription::{ResourceSubscriptionChannels, Subscribable, SubscriptionEvent};

pub type RelayedSubscriptionEvents = ResourceSubscriptionChannels;

pub enum ResourcesTransaction<'transaction> {
    Persistent(PersistentResourcesTransaction<'transaction>),
    Volatile(VolatileResourcesTransaction<'transaction>),
}
impl<'transaction> ResourcesTransaction<'transaction> {
    pub fn persistent(transaction: PersistentResourcesTransaction<'transaction>) -> Self {
        Self::Persistent(transaction)
    }
    pub fn volatile(transaction: VolatileResourcesTransaction<'transaction>) -> Self {
        Self::Volatile(transaction)
    }

    pub fn into_relayed_subscription_events(self) -> &'transaction mut RelayedSubscriptionEvents {
        match self {
            ResourcesTransaction::Persistent(transaction) => transaction.relayed_subscription_events,
            ResourcesTransaction::Volatile(transaction) => transaction.relayed_subscription_events,
        }
    }
}
impl<'transaction> ResourcesStorageApi for ResourcesTransaction<'transaction> {
    fn insert<R>(&mut self, id: R::Id, resource: R) -> PersistenceResult<()>
    where R: Resource + Persistable + Subscribable {
        match self {
            ResourcesTransaction::Persistent(transaction) => {
                let result = transaction.insert(id.clone(), resource.clone());
                if result.is_ok() {
                    transaction.relayed_subscription_events
                        .notify(SubscriptionEvent::Inserted { id, value: resource })
                        .expect("should successfully queue notification about resource insertion during transaction");
                }
                result
            }
            ResourcesTransaction::Volatile(transaction) => {
                let result = transaction.insert(id.clone(), resource.clone());
                if result.is_ok() {
                    transaction.relayed_subscription_events
                        .notify(SubscriptionEvent::Inserted { id, value: resource })
                        .expect("should successfully queue notification about resource insertion during transaction");
                }
                result
            }
        }
    }

    fn remove<R>(&mut self, id: R::Id) -> PersistenceResult<Option<R>>
    where R: Resource + Persistable {
        match self {
            ResourcesTransaction::Persistent(transaction) => transaction.remove(id),
            ResourcesTransaction::Volatile(transaction) => transaction.remove(id),
        }
    }

    fn get<R>(&self, id: R::Id) -> PersistenceResult<Option<R>>
    where R: Resource + Persistable + Clone {
        match &self {
            ResourcesTransaction::Persistent(transaction) => transaction.get(id),
            ResourcesTransaction::Volatile(transaction) => transaction.get(id),
        }
    }

    fn list<R>(&self) -> PersistenceResult<Vec<R>>
    where R: Resource + Persistable + Clone {
        match &self {
            ResourcesTransaction::Persistent(transaction) => transaction.list(),
            ResourcesTransaction::Volatile(transaction) => transaction.list(),
        }
    }
}
