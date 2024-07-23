use std::sync::Arc;

use config::Config;
use http::{HeaderMap, HeaderValue};
use oauth2::HttpRequest;
use openidconnect::{ClientName, ClientUrl};
use openidconnect::core::{CoreClientRegistrationRequest, CoreGrantType};
use openidconnect::registration::EmptyAdditionalClientMetadata;
use serde::Deserialize;
use tracing::error;
use url::Url;
use opendut_types::resources::Id;
use opendut_types::util::net::{ClientCredentials, ClientId, ClientSecret};

use crate::confidential::client::{ConfidentialClient, ConfidentialClientRef};
use crate::registration::config::RegistrationClientConfig;
use crate::registration::error::WrappedClientRegistrationError;

pub type RegistrationClientRef = Arc<RegistrationClient>;

pub const DEVICE_REDIRECT_URL: &str = "http://localhost:12345/device";

#[derive(Debug)]
pub struct RegistrationClient {
    pub inner: ConfidentialClientRef,
    pub config: RegistrationClientConfig,
}

#[derive(thiserror::Error, Debug)]
pub enum RegistrationClientError {
    #[error("Invalid configuration:\n  {error}")]
    InvalidConfiguration {
        error: String,
    },
    #[error("Failed request: {error}.\nCause: {cause}")]
    RequestError {
        error: String,
        cause: Box<dyn std::error::Error + Send + Sync>,  // RequestTokenError<OidcClientError<reqwest::Error>, BasicErrorResponse>
    },
    #[error("Failed to register new client: {message}\nCause: {cause}")]
    ClientParameter {
        message: String,
        cause: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Failed to register new client. Cause: {cause}")]
    Registration {
        cause: WrappedClientRegistrationError,
    },
    #[error("Client could not be found")]
    ClientNotFound,
    #[error("Following clients could not be deleted: {client_ids}")]
    ClientDeletionError {
        client_ids: String
    },
}


impl RegistrationClient {
    pub async fn from_settings(settings: &Config) -> Result<Option<RegistrationClientRef>, RegistrationClientError> {
        let inner = ConfidentialClient::from_settings(settings).await
            .map_err(|error| RegistrationClientError::InvalidConfiguration { error: error.to_string() })?;
        match inner {
            None => {
                // Authentication is disabled, ergo no registration client is needed
                Ok(None)
            }
            Some(inner) => {
                let registration_config = RegistrationClientConfig::from_settings(settings, &inner)?;

                Ok(Some(Self::new(registration_config, inner)))
            }
        }
    }
    
    pub fn new(registration_client_config: RegistrationClientConfig, inner: ConfidentialClientRef) -> RegistrationClientRef {
        Arc::new(Self {
            inner,
            config: registration_client_config,
        })
    }

    pub async fn register_new_client_for_user(&self, resource_id: Id, user_id: String) -> Result<ClientCredentials, RegistrationClientError> {
        match self.config.peer_credentials.clone() {
            Some(peer_credentials) => {
                Ok(peer_credentials)
            }
            None => {
                let access_token = self.inner.get_token().await
                    .map_err(|error| RegistrationClientError::RequestError { error: error.to_string(), cause: Box::new(error) })?;
                let additional_metadata = EmptyAdditionalClientMetadata {};
                let redirect_uris = vec![self.config.device_redirect_url.clone()];
                let grant_types = vec![CoreGrantType::ClientCredentials];
                let request: CoreClientRegistrationRequest =
                    openidconnect::registration::ClientRegistrationRequest::new(redirect_uris, additional_metadata)
                        .set_grant_types(Some(grant_types));
                let registration_url = self.config.registration_url.clone();

                let client_name: ClientName = ClientName::new(resource_id.to_string());
                let resource_uri = self.config.client_home_base_url.resource_url(resource_id, user_id)
                    .map_err(|error| RegistrationClientError::ClientParameter {
                        message: format!("Failed to create resource url for client: {:?}", error),
                        cause: Box::new(error),
                    })?;
                let client_home_uri = ClientUrl::new(String::from(resource_uri))
                    .map_err(|error| RegistrationClientError::ClientParameter {
                        message: format!("Failed to create client home url: {:?}", error),
                        cause: Box::new(error),
                    })?;
                let response = request
                    .set_initial_access_token(Some(access_token.oauth_token()))
                    .set_client_name(Some(
                        vec![(None, client_name)]
                            .into_iter()
                            .collect(),
                    ))
                    .set_client_uri(Some(vec![(None, client_home_uri)]
                        .into_iter()
                        .collect()))
                    .register_async(&registration_url, move |request| {
                        self.inner.reqwest_client.async_http_client(request)
                    }).await;
                match response {
                    Ok(response) => {
                        let client_id = response.client_id();
                        let client_secret = response.client_secret().expect("Confidential client required!");

                        Ok(ClientCredentials {
                            client_id: ClientId(client_id.to_string()),
                            client_secret: ClientSecret(client_secret.secret().to_string()),
                        })
                    }
                    Err(error) => {
                        Err(RegistrationClientError::Registration { cause: WrappedClientRegistrationError(error) })
                    }
                }
            }
        }
    }
    
    pub async fn list_clients(&self, resource_id: Id) -> Result<Clients, RegistrationClientError> {
        let enumerate_clients_uri = self.config.issuer_admin_url.join("clients/")
            .map_err(|cause| RegistrationClientError::InvalidConfiguration { error: format!("Invalid admin api endpoint for issuer. {}", cause) })?;
        let request = self.create_http_request_with_auth_token(&enumerate_clients_uri, http::Method::GET).await?;
        
        let response = self.inner.reqwest_client.async_http_client(request)
            .await;
        match response { 
            Ok(response) => {
                 let clients: Clients = serde_json::from_slice(&response.body).unwrap();
                 let filtered_clients = clients.value().into_iter().filter(|client| client.base_url.clone().is_some_and(|url| url.contains(&resource_id.value().to_string()))).collect::<Vec<Client>>();
    
                 Ok(Clients(filtered_clients))
            }
            Err(error) => {
                 Err(RegistrationClientError::RequestError { error: "OIDC client list request failed!".to_string(), cause: Box::new(error) })
            }
        }
    }

    pub async fn delete_client(&self, resource_id: Id) -> Result<Clients, RegistrationClientError> {
        let clients = self.list_clients(resource_id).await?;
        
        let mut failed_deletion_clients = Vec::new();
        
        for client in clients.value() {
            let client_uri = format!("clients/{}", client.client_id);
            let delete_client_url = self.config.issuer_admin_url.join(&client_uri)
                .map_err(|cause| RegistrationClientError::InvalidConfiguration { error: format!("Invalid admin api endpoint for issuer. {}", cause) })?;

            let request = self.create_http_request_with_auth_token(&delete_client_url, http::Method::DELETE).await?;

            let response = self.inner.reqwest_client.async_http_client(request)
                .await;

            if response.is_err() {
                failed_deletion_clients.push(client.client_id);
            }
        }
        
        if failed_deletion_clients.is_empty() {
            Ok(clients)
        } else {
            Err( RegistrationClientError::ClientDeletionError { client_ids: failed_deletion_clients.join(",") } )
        }
    }

    async fn create_http_request_with_auth_token(&self, issuer_remote_url: &Url, http_method: http::Method) -> Result<HttpRequest, RegistrationClientError> {
        let mut headers = HeaderMap::new();
        let access_token = self.inner.get_token().await
            .map_err(|error| RegistrationClientError::RequestError { error: error.to_string(), cause: error.into() })?;
        let bearer_header = format!("Bearer {}", access_token);
        let access_token_value = HeaderValue::from_str(&bearer_header)
            .map_err(|error| RegistrationClientError::InvalidConfiguration { error: error.to_string() })?;
        headers.insert(http::header::AUTHORIZATION, access_token_value);

        Ok(HttpRequest {
            method: http_method,
            url: issuer_remote_url.clone(),
            headers,
            body: vec![],
        })
    }
}

#[derive(Deserialize, Clone)]
pub struct Clients(Vec<Client>);

impl Clients {
    pub fn value(&self) -> Vec<Client> {
        self.0.clone()
    }
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub client_id: String,
    base_url: Option<String>,
}
