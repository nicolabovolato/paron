mod bootstrap;
pub mod cli;
mod data_dir;
mod paron;
mod run;
mod users;

use std::net::{Ipv4Addr, Ipv6Addr};

pub fn local_hosts() -> [url::Host; 3] {
    [
        url::Host::Domain("localhost".to_owned()),
        url::Host::Ipv4(Ipv4Addr::LOCALHOST),
        url::Host::Ipv6(Ipv6Addr::LOCALHOST),
    ]
}
