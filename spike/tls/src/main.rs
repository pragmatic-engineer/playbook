use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Proxy};

fn main() {
    let url = std::env::args().nth(1).unwrap_or_else(|| {
        "https://api.github.com/repos/pragmatic-engineer/playbook/releases/latest".to_string()
    });
    if std::env::var("SPIKE_SHOW_PROXY").is_ok() {
        println!("Proxy::try_from_env() = {:?}", Proxy::try_from_env());
    }
    let agent: Agent = Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent("tls-spike")
        .build()
        .into();
    match agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(mut resp) => {
            let status = resp.status();
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            let tag = body
                .split("\"tag_name\":\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("<none>");
            println!("TLS outcome: OK, status {status}, tag_name={tag}");
        }
        Err(e) => {
            println!("TLS outcome: ERROR");
            println!("  Display: {e}");
            println!("  Debug:   {e:?}");
        }
    }
}
