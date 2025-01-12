pub use crate::resources::subscription::SubscriptionEvent;

use crate::persistence::error::PersistenceResult;
use crate::persistence::resources::Persistable;
use crate::resources::storage::{PersistenceOptions, ResourcesStorageApi};
use crate::resources::subscription::{ResourceSubscriptionChannel, ResourceSubscriptionChannels, Subscribable, Subscription};
use crate::resources::transaction::RelayedSubscriptionEvents;
use crate::resources::{storage, Resource, Resources, ResourcesTransaction};
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockWriteGuard};

pub type ResourcesManagerRef = Arc<ResourcesManager>;

pub struct ResourcesManager {
    state: RwLock<State>,
}

struct State {
    resources: Resources,
    subscribers: ResourceSubscriptionChannels,
}

impl ResourcesManager {

    pub async fn create(storage_options: PersistenceOptions) -> Result<ResourcesManagerRef, storage::ConnectionError> {
        let resources = Resources::connect(storage_options).await?;
        let subscribers = ResourceSubscriptionChannels::default();

        Ok(Arc::new(Self {
            state: RwLock::new(State { resources, subscribers }),
        }))
    }

    pub async fn insert<R>(&self, id: R::Id, resource: R) -> PersistenceResult<()>
    where R: Resource + Persistable + Subscribable {
        let mut state = self.state.write().await;
        
        let (result, relayed_subscription_events) = state.resources.transaction(|transaction| {
            transaction.insert(id.clone(), resource.clone())
        })?;
        Self::send_relayed_subscription_events(relayed_subscription_events, &mut state).await;
        result
    }

    pub async fn remove<R>(&self, id: R::Id) -> PersistenceResult<Option<R>>
    where R: Resource + Persistable {
        let mut state = self.state.write().await;
        let (result, relayed_subscription_events) = state.resources.transaction(move |transaction| {
            transaction.remove(id)
        })?;
        Self::send_relayed_subscription_events(relayed_subscription_events, &mut state).await;
        result
    }

    pub async fn get<R>(&self, id: R::Id) -> PersistenceResult<Option<R>>
    where R: Resource + Persistable + Clone {
        let state = self.state.read().await;
        state.resources.get(id)
    }

    pub async fn list<R>(&self) -> PersistenceResult<Vec<R>>
    where R: Resource + Persistable + Clone {
        let state = self.state.read().await;
        state.resources.list()
    }

    pub async fn resources<F, T>(&self, f: F) -> PersistenceResult<T>
    where F: FnOnce(&Resources) -> PersistenceResult<T> {
        let state = self.state.read().await;
        f(&state.resources)
    }

    /// Allows grouping modifications to the database. This does multiple things:
    /// - Opens a database transaction and then either commits it, or rolls it back when you return an `Err` out of the closure.
    /// - Acquires the lock for the database mutex and keeps it until the end of the closure.
    /// - Groups the async calls, so we only have to await at the end.
    pub async fn resources_mut<F, T, E>(&self, f: F) -> PersistenceResult<Result<T, E>>
    where
        F: FnOnce(&mut ResourcesTransaction) -> Result<T, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let mut state = self.state.write().await;
        let (result, relayed_subscription_events) = state.resources.transaction(move |transaction| {
            f(transaction)
        })?;
        Self::send_relayed_subscription_events(relayed_subscription_events, &mut state).await;
        Ok(result)
    }

    pub async fn subscribe<R>(&self) -> Subscription<R>
    where R: Resource + Subscribable {
        let mut state = self.state.write().await;
        state.subscribers.subscribe()
    }

    async fn send_relayed_subscription_events(
        relayed_subscription_events: RelayedSubscriptionEvents,
        state: &mut RwLockWriteGuard<'_, State>,
    ) {
        let ResourceSubscriptionChannels {
            cluster_configuration,
            cluster_deployment,
            old_peer_configuration,
            peer_configuration,
            peer_descriptor,
            peer_state
        } = relayed_subscription_events;

        async fn notify_for_relayed_subscription_events_on_channel<R: Resource + Subscribable + Clone>(
            channel: ResourceSubscriptionChannel<R>,
            state: &mut RwLockWriteGuard<'_, State>,
        ) {
            let (_, mut receiver) = channel;
            while let Ok(event) = receiver.try_recv() {
                state.subscribers
                    .notify(event)
                    .expect("should successfully send notification about event during resource transaction");
            }
        }

        notify_for_relayed_subscription_events_on_channel(cluster_configuration, state).await;
        notify_for_relayed_subscription_events_on_channel(cluster_deployment, state).await;
        notify_for_relayed_subscription_events_on_channel(old_peer_configuration, state).await;
        notify_for_relayed_subscription_events_on_channel(peer_configuration, state).await;
        notify_for_relayed_subscription_events_on_channel(peer_descriptor, state).await;
        notify_for_relayed_subscription_events_on_channel(peer_state, state).await;
    }
}


#[cfg(test)]
impl ResourcesManager {
    pub fn new_in_memory() -> ResourcesManagerRef {
        let resources = futures::executor::block_on(
            Resources::connect(PersistenceOptions::Disabled)
        )
        .expect("Creating in-memory storage for tests should not fail");

        let subscribers = ResourceSubscriptionChannels::default();

        Arc::new(Self {
            state: RwLock::new(State { resources, subscribers }),
        })
    }

    async fn contains<R>(&self, id: R::Id) -> bool
    where R: Resource + Clone {
        let state = self.state.read().await;
        state.resources.contains::<R>(id).await
    }

    async fn is_empty(&self) -> bool {
        let state = self.state.read().await;
        state.resources.is_empty().await
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;
    use std::ops::Not;
    use std::vec;

    use googletest::prelude::*;

    use super::*;
    use opendut_types::cluster::{ClusterConfiguration, ClusterId, ClusterName};
    use opendut_types::peer::executor::{container::{ContainerCommand, ContainerImage, ContainerName, Engine}, ExecutorDescriptor, ExecutorDescriptors, ExecutorId, ExecutorKind};
    use opendut_types::peer::{PeerDescriptor, PeerId, PeerLocation, PeerName, PeerNetworkDescriptor};
    use opendut_types::topology::Topology;
    use opendut_types::util::net::{NetworkInterfaceConfiguration, NetworkInterfaceDescriptor, NetworkInterfaceId, NetworkInterfaceName};

    #[tokio::test]
    async fn test() -> Result<()> {

        let testee = ResourcesManager::new_in_memory();

        let peer_resource_id = PeerId::random();
        let peer = PeerDescriptor {
            id: peer_resource_id,
            name: PeerName::try_from("TestPeer")?,
            location: PeerLocation::try_from("Ulm").ok(),
            network: PeerNetworkDescriptor {
                interfaces: vec![
                    NetworkInterfaceDescriptor {
                        id: NetworkInterfaceId::random(),
                        name: NetworkInterfaceName::try_from("eth0")?,
                        configuration: NetworkInterfaceConfiguration::Ethernet,
                    },
                ],
                bridge_name: Some(NetworkInterfaceName::try_from("br-opendut-1")?),
            },
            topology: Topology::default(),
            executors: ExecutorDescriptors {
                executors: vec![
                    ExecutorDescriptor {
                        id: ExecutorId::random(),
                        kind: ExecutorKind::Container {
                            engine: Engine::Docker,
                            name: ContainerName::Empty,
                            image: ContainerImage::try_from("testUrl")?,
                            volumes: vec![],
                            devices: vec![],
                            envs: vec![],
                            ports: vec![],
                            command: ContainerCommand::Default,
                            args: vec![],
                        },
                        results_url: None,
                    }
                ],
            }
        };

        let cluster_resource_id = ClusterId::random();
        let cluster_configuration = ClusterConfiguration {
            id: cluster_resource_id,
            name: ClusterName::try_from("ClusterX032")?,
            leader: peer.id,
            devices: HashSet::new(),
        };

        assert!(testee.is_empty().await);

        testee.insert(peer_resource_id, Clone::clone(&peer)).await?;

        assert!(testee.is_empty().await.not());

        testee.insert(cluster_resource_id, Clone::clone(&cluster_configuration)).await?;

        assert_that!(testee.get::<PeerDescriptor>(peer_resource_id).await?, some(eq(&peer)));
        assert_that!(testee.get::<ClusterConfiguration>(cluster_resource_id).await?, some(eq(&cluster_configuration)));

        assert!(testee.contains::<PeerDescriptor>(peer_resource_id).await);

        assert_that!(testee.get::<PeerDescriptor>(PeerId::random()).await?, none());

        assert_that!(testee.remove::<PeerDescriptor>(peer_resource_id).await?, some(eq(&peer)));

        testee.insert(peer_resource_id, Clone::clone(&peer)).await?;

        assert_that!(testee.get::<PeerDescriptor>(peer_resource_id).await?, some(eq(&peer)));

        testee.resources(|resources| {
            resources.list::<ClusterConfiguration>()?
                .into_iter()
                .for_each(|cluster| {
                    assert_that!(cluster, eq(&cluster_configuration));
                });
            Ok(())
        }).await?;

        Ok(())
    }
}
