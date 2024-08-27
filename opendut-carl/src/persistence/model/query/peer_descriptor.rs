use diesel::{ExpressionMethods, PgConnection, QueryDsl, RunQueryDsl, SelectableHelper};
use uuid::Uuid;

use crate::persistence::database::schema;
use crate::persistence::error::{PersistenceError, PersistenceResult};
use crate::persistence::model::query;
use crate::persistence::model::query::Filter;
use opendut_types::peer::executor::ExecutorDescriptors;
use opendut_types::peer::{PeerDescriptor, PeerId, PeerLocation, PeerName, PeerNetworkDescriptor};
use opendut_types::topology::Topology;
use opendut_types::util::net::NetworkInterfaceName;

pub fn insert(peer_descriptor: PeerDescriptor, connection: &mut PgConnection) -> PersistenceResult<()> {
    let PeerDescriptor { id: peer_id, name, location, network, topology, executors } = peer_descriptor;
    let PeerNetworkDescriptor { interfaces, bridge_name } = network;

    insert_persistable(PersistablePeerDescriptor {
        peer_id: peer_id.uuid,
        name: name.value(),
        location: location.map(|location| location.value()),
        network_bridge_name: bridge_name.map(|name| name.name()),
    }, connection)?;

    for interface in interfaces {
        query::network_interface_descriptor::insert(interface, peer_id, connection)?;
    }

    let Topology { devices } = topology;

    for device in devices {
        query::device_descriptor::insert(device, connection)?;
    }

    for executor in executors.executors {
        query::executor_descriptor::insert_into_database(executor, peer_id, connection)?;
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, diesel::Queryable, diesel::Selectable, diesel::Insertable, diesel::AsChangeset)]
#[diesel(table_name = schema::peer_descriptor)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct PersistablePeerDescriptor {
    pub peer_id: Uuid,
    pub name: String,
    pub location: Option<String>,
    pub network_bridge_name: Option<String>,
}
fn insert_persistable(persistable: PersistablePeerDescriptor, connection: &mut PgConnection) -> PersistenceResult<()> {
    diesel::insert_into(schema::peer_descriptor::table)
        .values(&persistable)
        .on_conflict(schema::peer_descriptor::peer_id)
        .do_update()
        .set(&persistable)
        .execute(connection)
        .map_err(|cause| PersistenceError::insert::<PeerDescriptor>(persistable.peer_id, cause))?;
    Ok(())
}

pub fn remove(peer_id: PeerId, connection: &mut PgConnection) -> PersistenceResult<Option<PeerDescriptor>> {
    let result = list(Filter::By(peer_id), connection)?
        .first().cloned();

    diesel::delete(
        schema::peer_descriptor::table
            .filter(schema::peer_descriptor::peer_id.eq(peer_id.uuid))
    )
    .execute(connection)
    .map_err(|cause| PersistenceError::remove::<PeerDescriptor>(peer_id.uuid, cause))?;

    Ok(result)
}

pub fn list(filter_by_peer_id: Filter<PeerId>, connection: &mut PgConnection) -> PersistenceResult<Vec<PeerDescriptor>> {
    let mut query = schema::peer_descriptor::table.into_boxed();

    if let Filter::By(peer_id) = filter_by_peer_id {
        query = query.filter(schema::peer_descriptor::peer_id.eq(peer_id.uuid));
    }

    let persistable_peer_descriptors = query
        .select(PersistablePeerDescriptor::as_select())
        .get_results(connection)
        .map_err(PersistenceError::list::<PeerDescriptor>)?;

    persistable_peer_descriptors.into_iter().map(|persistable| {
        let PersistablePeerDescriptor { peer_id, name, location, network_bridge_name } = persistable;

        let peer_id = PeerId::from(peer_id);

        let name = PeerName::try_from(name)
            .map_err(|cause| PersistenceError::get::<PeerDescriptor>(peer_id.uuid, cause))?;

        let location = location.map(PeerLocation::try_from).transpose()
            .map_err(|cause| PersistenceError::get::<PeerDescriptor>(peer_id.uuid, cause))?;

        let network_bridge_name = network_bridge_name.map(NetworkInterfaceName::try_from).transpose()
            .map_err(|cause| PersistenceError::get::<PeerDescriptor>(peer_id.uuid, cause))?;

        let network_interfaces = query::network_interface_descriptor::list_filtered_by_peer(peer_id, connection)?;

        let devices = query::device_descriptor::list_filtered_by_peer(peer_id, connection)?;

        let executors = query::executor_descriptor::list_filtered_by_peer(peer_id, connection)?;

        Ok(PeerDescriptor {
            id: peer_id,
            name,
            location,
            network: PeerNetworkDescriptor {
                interfaces: network_interfaces,
                bridge_name: network_bridge_name,
            },
            topology: Topology {
                devices,
            },
            executors: ExecutorDescriptors { executors },
        })
    })
    .collect::<PersistenceResult<Vec<_>>>()
    .map_err(|cause|
        PersistenceError::list::<PeerDescriptor>(cause)
            .context("Failed to convert from database values to PeerDescriptor.")
    )
}
