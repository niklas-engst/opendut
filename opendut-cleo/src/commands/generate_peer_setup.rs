use opendut_carl_api::carl::CarlClient;
use opendut_types::peer::PeerId;
use uuid::Uuid;

/// Generate a string to setup a peer
#[derive(clap::Parser)]
pub struct GeneratePeerSetupCli {
    ///PeerID
    #[arg(short, long)]
    id: Uuid,
}

impl GeneratePeerSetupCli {
    //TODO: what happens if peer with the ID is already set up?
    pub async fn execute(self, carl: &mut CarlClient) -> crate::Result<()> {
        let peer_id = PeerId::from(self.id);
        let created_setup = carl
            .peers
            .create_peer_setup(peer_id)
            .await
            .map_err(|error| format!("Could not create peer setup.\n  {}", error))?;

        match created_setup.encode() {
            Ok(setup_key) => {
                println!("{}", setup_key);
            }
            Err(_) => {
                println!("Could not configure setup key...")
            }
        }
        Ok(())
    }
}
