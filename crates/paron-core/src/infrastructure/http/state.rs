use std::sync::Arc;

use crate::application::audit::AuditReader;
use crate::application::identity::use_cases::{
    AccountRecovery, Authentication, CredentialManagement, UserManagement,
};
use crate::application::machine::{MachineConnections, MachineEnrollment, MachineManagement};
use crate::infrastructure::http::public_origins::PublicOrigins;

#[derive(Clone)]
pub struct Deps {
    pub machines: Arc<dyn MachineManagement>,
    pub machine_connections: Arc<dyn MachineConnections>,
    pub machine_enrollment: Arc<dyn MachineEnrollment>,
    pub authentication: Arc<dyn Authentication>,
    pub credentials: Arc<dyn CredentialManagement>,
    pub account_recovery: Arc<dyn AccountRecovery>,
    pub users: Arc<dyn UserManagement>,
    pub audit: Arc<dyn AuditReader>,
    pub public_origins: Arc<PublicOrigins>,
    pub config: super::Config,
}
