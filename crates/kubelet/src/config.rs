//! Configuration for a Kubelet
//!
//! You must provide a Config when instantiating a kubelet. Although you can create a Config
//! directly, it is usually easier to use one of the following functions:
//!
//! * [`Config::default_config`] - use the defaults for everything
//! * [`Config::new_from_file`] - use the values in the specified file
//! * [`Config::new_from_flags`] - use the values specified on the command line or in
//!   environment variables (requires you to turn on the "cli" feature)
//! * [`Config::new_from_file_and_flags`] - use the values specified on the command line
//!   or in environment variables, but falling back to the specified configuration file
//!   (requires you to turn on the "cli" feature)

use std::iter::FromIterator;
use std::net::IpAddr;
use std::net::ToSocketAddrs;
use std::path::PathBuf;

#[cfg(feature = "cli")]
use structopt::StructOpt;

use std::collections::HashMap;

use serde::Deserialize;

const DEFAULT_PORT: u16 = 3000;
const DEFAULT_MAX_PODS: u16 = 110;
const BOOTSTRAP_FILE: &str = "/etc/kubernetes/bootstrap-kubelet.conf";

/// The configuration needed for a kubelet to run properly.
///
/// This can be configured manually in your code or if you are exposing a CLI, use the
/// [`Config::new_from_flags`] (this requires the "cli" feature to
/// be enabled).
///
/// Use [`Config::default_config`] to generate a config with all
/// of the default values set.
#[derive(Clone, Debug)]
pub struct Config {
    /// The ip address the node is exposed on
    pub node_ip: IpAddr,
    /// The hostname of the node
    pub hostname: String,
    /// The node's name
    pub node_name: String,
    /// The Kubelet server configuration
    pub server_config: ServerConfig,
    /// The directory where the Kubelet will store data
    pub data_dir: PathBuf,
    /// Labels to add when registering the node in the cluster
    pub node_labels: HashMap<String, String>,
    /// The maximum pods for this kubelet (reported to apiserver)
    pub max_pods: u16,
    /// The location of the tls bootstrapping file
    pub bootstrap_file: PathBuf,
}
/// The configuration for the Kubelet server.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The ip address the Kubelet server is running on
    pub addr: IpAddr,
    /// The port the Kubelet server is running on
    pub port: u16,
    /// Path to kubelet TLS certificate.
    pub tls_cert_file: PathBuf,
    /// Path to kubelet TLS private key.
    pub tls_private_key_file: PathBuf,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigBuilder {
    // Some -> Ok(v) = it was present and the value parsed as v
    //      -> Err(e) = it was present but bad - e described the problem
    // None = it wasn't present
    #[serde(
        default,
        rename = "nodeIP",
        deserialize_with = "try_deserialize_ip_addr"
    )]
    pub node_ip: Option<anyhow::Result<IpAddr>>,
    #[serde(default, rename = "hostname")]
    pub hostname: Option<String>,
    #[serde(default, rename = "nodeName")]
    pub node_name: Option<String>,
    #[serde(default, rename = "dataDir")]
    pub data_dir: Option<PathBuf>,
    #[serde(default, rename = "nodeLabels")]
    pub node_labels: Option<HashMap<String, String>>,
    #[serde(default, rename = "maxPods", deserialize_with = "try_deserialize_u16")]
    pub max_pods: Option<anyhow::Result<u16>>,
    #[serde(
        default,
        rename = "listenerAddress",
        deserialize_with = "try_deserialize_ip_addr"
    )]
    pub server_addr: Option<anyhow::Result<IpAddr>>,
    #[serde(
        default,
        rename = "listenerPort",
        deserialize_with = "try_deserialize_u16"
    )]
    pub server_port: Option<anyhow::Result<u16>>,
    #[serde(default, rename = "tlsCertificateFile")]
    pub server_tls_cert_file: Option<PathBuf>,
    #[serde(default, rename = "tlsPrivateKeyFile")]
    pub server_tls_private_key_file: Option<PathBuf>,
}

struct ConfigBuilderFallbacks {
    hostname: fn() -> String,
    data_dir: fn() -> PathBuf,
    cert_path: fn(data_dir: &PathBuf) -> PathBuf,
    key_path: fn(data_dir: &PathBuf) -> PathBuf,
    node_ip: fn(hostname: &mut String, preferred_ip_family: &IpAddr) -> IpAddr,
}

impl Config {
    /// Returns a Config object set with all of the defaults.
    ///
    /// Useful for cases when you don't want to set most of the values yourself. The
    /// preferred_ip_family argument takes an IpAddr that is either V4 or V6 to
    /// indicate the preferred IP family to use for defaults
    pub fn default_config(preferred_ip_family: &IpAddr) -> anyhow::Result<Self> {
        let hostname = default_hostname()?;
        let data_dir = default_data_dir()?;
        let tls_cert_file = default_cert_path(&data_dir);
        let tls_private_key_file = default_key_path(&data_dir);
        Ok(Config {
            node_ip: default_node_ip(&mut hostname.clone(), preferred_ip_family)?,
            node_name: sanitize_hostname(&hostname),
            node_labels: HashMap::new(),
            hostname,
            data_dir,
            max_pods: DEFAULT_MAX_PODS,
            bootstrap_file: PathBuf::from(BOOTSTRAP_FILE),
            server_config: ServerConfig {
                addr: match preferred_ip_family {
                    // Just unwrap these because they are programmer error if they
                    // don't parse
                    IpAddr::V4(_) => "0.0.0.0".parse().unwrap(),
                    IpAddr::V6(_) => "::".parse().unwrap(),
                },
                port: DEFAULT_PORT,
                tls_cert_file,
                tls_private_key_file,
            },
        })
    }

    fn new_from_builder(builder: ConfigBuilder) -> Self {
        let fallbacks = ConfigBuilderFallbacks {
            hostname: || default_hostname().expect("unable to get default hostname"),
            data_dir: || default_data_dir().expect("unable to get default data directory"),
            cert_path: default_cert_path,
            key_path: default_key_path,
            node_ip: |hn, ip| default_node_ip(hn, ip).expect("unable to get default node IP"),
        };
        ConfigBuilder::build(builder, fallbacks).unwrap()
    }

    /// Parses the specified config file and sets the proper defaults.
    /// If the specified file does not exist, this function panics.
    /// It is up to callers of the function to ensure any file they specify exists.
    pub fn new_from_file(filename: PathBuf) -> Self {
        let builder = ConfigBuilder::from_config_file(filename).unwrap();
        Config::new_from_builder(builder)
    }

    /// Parses all command line flags and sets the proper defaults. The version
    /// of your application should be passed to set the proper version for the CLI
    #[cfg(any(feature = "cli", feature = "docs"))]
    #[cfg_attr(feature = "docs", doc(cfg(feature = "cli")))]
    pub fn new_from_flags(version: &str) -> Self {
        let app = Opts::clap().version(version);
        let opts = Opts::from_clap(&app.get_matches());
        let builder = ConfigBuilder::from_opts(opts);
        Config::new_from_builder(builder)
    }

    /// Parses the specified config file (or the default config file if no file is
    /// specified and the default config file exists) and command line flags and
    /// sets the proper defaults. The version of your application should be passed
    /// to set the proper version for CLI flags.
    ///
    /// If the config file is specified but does not exist, this function panics.
    /// It is up to callers of the function to ensure any file they specify exists.
    /// If no file is specified, and the default config file does not exist, then
    /// this is not an error and the configuration is determined solely from the
    /// CLI flags.
    #[cfg(any(feature = "cli", feature = "docs"))]
    #[cfg_attr(feature = "docs", doc(cfg(feature = "cli")))]
    pub fn new_from_file_and_flags(version: &str, config_file_path: Option<PathBuf>) -> Self {
        match config_file_path {
            None => {
                let default_path = default_config_file_path();
                if default_path.exists() {
                    Config::new_from_file_and_flags_impl(version, default_path)
                } else {
                    Config::new_from_flags(version)
                }
            }
            Some(path) => Config::new_from_file_and_flags_impl(version, path),
        }
    }

    #[cfg(any(feature = "cli", feature = "docs"))]
    #[cfg_attr(feature = "docs", doc(cfg(feature = "cli")))]
    fn new_from_file_and_flags_impl(version: &str, config_file_path: PathBuf) -> Self {
        // TODO: reduce duplication
        let app = Opts::clap().version(version);
        let opts = Opts::from_clap(&app.get_matches());
        let cli_builder = ConfigBuilder::from_opts(opts);

        let config_file_builder = ConfigBuilder::from_config_file(config_file_path);

        let builder = config_file_builder.unwrap().with_override(cli_builder); // if the config file is actually malformed then we should halt even if there are CLI values
        Config::new_from_builder(builder)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::default_config(
            &"127.0.0.1"
                .parse()
                .expect("Could not parse hardcoded address"),
        )
        .expect("Could not create default config")
    }
}

fn ok_result_of<T>(value: Option<T>) -> Option<anyhow::Result<T>> {
    value.map(Ok)
}

impl ConfigBuilder {
    fn from_opts(opts: Opts) -> Self {
        let node_labels: Vec<(String, String)> = opts
            .node_labels
            .iter()
            .filter_map(|i| split_one_label(i))
            .collect();

        ConfigBuilder {
            node_ip: ok_result_of(opts.node_ip),
            node_name: opts.node_name,
            node_labels: if node_labels.is_empty() {
                None
            } else {
                Some(HashMap::from_iter(node_labels))
            },
            hostname: opts.hostname,
            data_dir: opts.data_dir,
            max_pods: ok_result_of(opts.max_pods),
            server_addr: ok_result_of(opts.addr),
            server_port: ok_result_of(opts.port),
            server_tls_cert_file: opts.tls_cert_file,
            server_tls_private_key_file: opts.tls_private_key_file,
        }
    }

    fn from_config_file(config_file_path: PathBuf) -> anyhow::Result<ConfigBuilder> {
        if !config_file_path.exists() {
            return Ok(ConfigBuilder::default());
        }
        let config_file = std::fs::File::open(config_file_path)?;
        ConfigBuilder::from_reader(config_file)
    }

    fn from_reader<R>(reader: R) -> anyhow::Result<ConfigBuilder>
    where
        R: std::io::Read,
    {
        serde_json::from_reader(reader).map_err(anyhow::Error::new)
    }

    fn with_override(self: Self, other: Self) -> Self {
        ConfigBuilder {
            node_ip: other.node_ip.or(self.node_ip),
            node_name: other.node_name.or(self.node_name),
            node_labels: other.node_labels.or(self.node_labels),
            hostname: other.hostname.or(self.hostname),
            data_dir: other.data_dir.or(self.data_dir),
            max_pods: other.max_pods.or(self.max_pods),
            server_addr: other.server_addr.or(self.server_addr),
            server_port: other.server_port.or(self.server_port),
            server_tls_cert_file: other.server_tls_cert_file.or(self.server_tls_cert_file),
            server_tls_private_key_file: other
                .server_tls_private_key_file
                .or(self.server_tls_private_key_file),
        }
    }

    fn build(self: Self, fallbacks: ConfigBuilderFallbacks) -> anyhow::Result<Config> {
        let empty_ip_addr = IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0));

        let hostname = self.hostname.unwrap_or_else(fallbacks.hostname);
        let data_dir = self.data_dir.unwrap_or_else(fallbacks.data_dir);
        let server_addr = self
            .server_addr
            .unwrap_or(Ok(empty_ip_addr))
            .map_err(|e| invalid_config_value_error(e, "server address"))?;
        let server_tls_cert_file = self
            .server_tls_cert_file
            .unwrap_or_else(|| (fallbacks.cert_path)(&data_dir));
        let server_tls_private_key_file = self
            .server_tls_private_key_file
            .unwrap_or_else(|| (fallbacks.key_path)(&data_dir));
        let server_port = self
            .server_port
            .unwrap_or(Ok(DEFAULT_PORT))
            .map_err(|e| invalid_config_value_error(e, "server port"))?;
        let node_ip = self
            .node_ip
            .unwrap_or_else(|| Ok((fallbacks.node_ip)(&mut hostname.clone(), &server_addr)))
            .map_err(|e| invalid_config_value_error(e, "node IP"))?;
        let node_name = self
            .node_name
            .unwrap_or_else(|| sanitize_hostname(&hostname));
        let max_pods = self
            .max_pods
            .unwrap_or(Ok(DEFAULT_MAX_PODS))
            .map_err(|e| invalid_config_value_error(e, "maximum pods"))?;

        let bootstrap_file = opts.bootstrap_file;

        Ok(Config {
            node_ip,
            node_name,
            node_labels: self.node_labels.unwrap_or_else(HashMap::new),
            hostname,
            data_dir,
            max_pods,
            bootstrap_file,
            server_config: ServerConfig {
                tls_cert_file: server_tls_cert_file,
                tls_private_key_file: server_tls_private_key_file,
                addr: server_addr,
                port: server_port,
            },
        })
    }
}

fn try_deserialize_ip_addr<'de, D>(d: D) -> Result<Option<anyhow::Result<IpAddr>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    let addr = s.parse::<IpAddr>().map_err(anyhow::Error::new);
    Ok(Some(addr))
}

fn try_deserialize_u16<'de, D>(d: D) -> Result<Option<anyhow::Result<u16>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let n = u16::deserialize(d).map_err(|e| anyhow::Error::msg(format!("{}", e)));
    Ok(Some(n))
}

/// CLI options that can be configured for Kubelet
///
/// These can be parsed from args using `Opts::into_app()`
#[derive(StructOpt, Clone, Debug)]
#[cfg(any(feature = "cli", feature = "docs"))]
#[cfg_attr(feature = "docs", doc(cfg(feature = "cli")))]
#[structopt(
    name = "krustlet",
    about = "A kubelet for running WebAssembly workloads"
)]
pub struct Opts {
    #[structopt(
        short = "a",
        long = "addr",
        env = "KRUSTLET_ADDRESS",
        help = "The address krustlet should listen on"
    )]
    addr: Option<IpAddr>,

    #[structopt(
        short = "p",
        long = "port",
        env = "KRUSTLET_PORT",
        help = "The port krustlet should listen on. Defaults to 3000"
    )]
    port: Option<u16>,

    #[structopt(
        long = "max-pods",
        env = "MAX_PODS",
        help = "The maximum pods for this kubelet (reported to apiserver). Defaults to 110"
    )]
    max_pods: Option<u16>,

    #[structopt(
        long = "tls-cert-file",
        env = "TLS_CERT_FILE",
        help = "The path to kubelet TLS certificate. Defaults to $KRUSTLET_DATA_DIR/config/krustlet.crt"
    )]
    tls_cert_file: Option<PathBuf>,

    #[structopt(
        long = "tls-private-key-file",
        env = "TLS_PRIVATE_KEY_FILE",
        help = "The path to kubelet TLS key. Defaults to $KRUSTLET_DATA_DIR/config/krustlet.key"
    )]
    tls_private_key_file: Option<PathBuf>,

    #[structopt(
        short = "n",
        long = "node-ip",
        env = "KRUSTLET_NODE_IP",
        help = "The IP address of the node registered with the Kubernetes master. Defaults to the IP address of the host name in DNS as a best effort try at a default"
    )]
    node_ip: Option<IpAddr>,

    #[structopt(
        long = "node-labels",
        env = "NODE_LABELS",
        use_delimiter = true,
        help = "Labels to add when registering the node in the cluster.
        Labels must be key=value pairs separated by ','.
        Labels in the 'kubernetes.io' namespace must begin with an allowed prefix
        (kubelet.kubernetes.io, node.kubernetes.io) or be in the specifically allowed set
        (beta.kubernetes.io/arch, beta.kubernetes.io/instance-type, beta.kubernetes.io/os,
        failure-domain.beta.kubernetes.io/region, failure-domain.beta.kubernetes.io/zone,
        failure-domain.kubernetes.io/region, failure-domain.kubernetes.io/zone,
        kubernetes.io/arch, kubernetes.io/hostname, kubernetes.io/instance-type,
        kubernetes.io/os)"
    )]
    node_labels: Vec<String>,

    #[structopt(
        long = "hostname",
        env = "KRUSTLET_HOSTNAME",
        help = "The hostname for this node, defaults to the hostname of this machine"
    )]
    hostname: Option<String>,

    #[structopt(
        long = "node-name",
        env = "KRUSTLET_NODE_NAME",
        help = "The name for this node in Kubernetes, defaults to the hostname of this machine"
    )]
    node_name: Option<String>,

    #[structopt(
        long = "data-dir",
        env = "KRUSTLET_DATA_DIR",
        help = "The data path (logs, container images, etc) for krustlet storage. Defaults to $HOME/.krustlet"
    )]
    data_dir: Option<PathBuf>,

    #[structopt(
        long = "bootstrap-file",
        env = "KRUSTLET_BOOTSTRAP_FILE",
        help = "The path to the bootstrap config",
        default_value = "/etc/kubernetes/bootstrap-kubelet.conf"
    )]
    bootstrap_file: PathBuf,
}

fn default_hostname() -> anyhow::Result<String> {
    Ok(hostname::get()?
        .into_string()
        .map_err(|_| anyhow::anyhow!("invalid utf-8 hostname string"))?)
}

fn default_data_dir() -> anyhow::Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Unable to get home directory"))?
        .join(".krustlet"))
}

// Some hostnames (particularly local ones) can have uppercase letters, which is
// disallowed by the DNS spec used in kubernetes naming. This sanitizes those
// names
fn sanitize_hostname(hostname: &str) -> String {
    // TODO: Are there other sanitation steps we should do here?
    hostname.to_lowercase()
}

// Attempt to get the node IP address in the following order (this follows the
// same pattern as the Kubernetes kubelet):
// 1. Lookup the IP from node name by DNS
// 2. Try to get the IP from the network interface used as default gateway
//    (unimplemented for now because it doesn't work across platforms)
fn default_node_ip(hostname: &mut String, preferred_ip_family: &IpAddr) -> anyhow::Result<IpAddr> {
    // NOTE: As of right now, we don't have cloud providers. In the future if
    // that is the case, we will need to add logic for looking up the IP and
    // hostname using the cloud provider as they do in the kubelet
    // To use the local resolver, we need to add a port to the hostname. Doesn't
    // matter which one, it just needs to be a valid socket address
    hostname.push_str(":80");
    Ok(hostname
        .to_socket_addrs()?
        .find(|i| {
            !i.ip().is_loopback()
                && !i.ip().is_multicast()
                && !i.ip().is_unspecified()
                && is_same_ip_family(&i.ip(), preferred_ip_family)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unable to find default IP address for node. Please specify a node IP manually"
            )
        })?
        .ip())
}

fn default_key_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("config/krustlet.key")
}

fn default_cert_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("config/krustlet.crt")
}

fn default_config_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".krustlet/config/config.json")
}

fn is_same_ip_family(first: &IpAddr, second: &IpAddr) -> bool {
    match first {
        IpAddr::V4(_) => second.is_ipv4(),
        IpAddr::V6(_) => second.is_ipv6(),
    }
}

fn split_one_label(in_string: &str) -> Option<(String, String)> {
    let mut splitter = in_string.splitn(2, '=');

    match splitter.next() {
        Some("") | None => None,
        Some(key) => match splitter.next() {
            Some(val) => Some((key.to_string(), val.to_string())),
            None => Some((key.to_string(), String::new())),
        },
    }
}

fn invalid_config_value_error(e: anyhow::Error, value_name: &str) -> anyhow::Error {
    let context = format!("invalid {} in configuration file: {}", value_name, e);
    e.context(context)
}

#[cfg(test)]
mod test {
    use super::*;

    fn builder_from_json_string(json: &str) -> anyhow::Result<ConfigBuilder> {
        ConfigBuilder::from_reader(json.as_bytes())
    }

    fn fallbacks() -> ConfigBuilderFallbacks {
        ConfigBuilderFallbacks {
            node_ip: |_, _| IpAddr::V4(std::net::Ipv4Addr::new(4, 4, 4, 4)),
            hostname: || "fallback-hostname".to_owned(),
            data_dir: || PathBuf::from("/fallback/data/dir"),
            cert_path: |_| PathBuf::from("/fallback/cert/path"),
            key_path: |_| PathBuf::from("/fallback/key/path"),
        }
    }

    #[test]
    fn config_file_inputs_are_respected_if_present() {
        let config_builder = builder_from_json_string(
            r#"{
            "listenerPort": 1234,
            "listenerAddress": "172.182.192.1",
            "hostname": "krusty-host",
            "dataDir": "/krusty/data/dir",
            "maxPods": 400,
            "nodeIP": "173.183.193.2",
            "nodeLabels": {
                "label1": "val1",
                "label2": "val2"
            },
            "nodeName": "krusty-node",
            "tlsCertificateFile": "/my/secure/cert.pfx",
            "tlsPrivateKeyFile": "/the/key"
        }"#,
        );
        let config = config_builder.unwrap().build(fallbacks()).unwrap();
        assert_eq!(config.server_config.port, 1234);
        assert_eq!(format!("{}", config.server_config.addr), "172.182.192.1");
        assert_eq!(
            config.server_config.tls_cert_file.to_string_lossy(),
            "/my/secure/cert.pfx"
        );
        assert_eq!(
            config.server_config.tls_private_key_file.to_string_lossy(),
            "/the/key"
        );
        assert_eq!(config.node_name, "krusty-node");
        assert_eq!(config.hostname, "krusty-host");
        assert_eq!(config.data_dir.to_string_lossy(), "/krusty/data/dir");
        assert_eq!(format!("{}", config.node_ip), "173.183.193.2");
        assert_eq!(config.max_pods, 400);
        assert_eq!(config.node_labels.len(), 2);
        assert_eq!(config.node_labels.get("label1"), Some(&("val1".to_owned())));
    }

    #[test]
    fn config_fallbacks_are_respected() {
        let config_builder = builder_from_json_string(
            r#"{
            "listenerPort": 2345,
            "listenerAddress": "173.183.193.2",
            "nodeLabels": {
                "label": "val"
            },
            "nodeName": "krustsome-node"
        }"#,
        );
        let config = config_builder.unwrap().build(fallbacks()).unwrap();
        assert_eq!(config.server_config.port, 2345);
        assert_eq!(format!("{}", config.server_config.addr), "173.183.193.2");
        assert_eq!(
            config.server_config.tls_cert_file.to_string_lossy(),
            "/fallback/cert/path"
        );
        assert_eq!(
            config.server_config.tls_private_key_file.to_string_lossy(),
            "/fallback/key/path"
        );
        assert_eq!(config.node_name, "krustsome-node");
        assert_eq!(config.hostname, "fallback-hostname");
        assert_eq!(config.data_dir.to_string_lossy(), "/fallback/data/dir");
        assert_eq!(format!("{}", config.node_ip), "4.4.4.4");
        assert_eq!(config.node_labels.get("label"), Some(&("val".to_owned())));
    }

    #[test]
    fn defaults_are_respected() {
        let config_builder = builder_from_json_string(
            r#"{
        }"#,
        );
        let config = config_builder.unwrap().build(fallbacks()).unwrap();
        assert_eq!(config.server_config.port, 3000);
        assert_eq!(config.max_pods, 110);
        assert_eq!(format!("{}", config.server_config.addr), "0.0.0.0");
        assert_eq!(
            config.server_config.tls_cert_file.to_string_lossy(),
            "/fallback/cert/path"
        );
        assert_eq!(
            config.server_config.tls_private_key_file.to_string_lossy(),
            "/fallback/key/path"
        );
        assert_eq!(config.node_name, "fallback-hostname");
        assert_eq!(config.hostname, "fallback-hostname");
        assert_eq!(config.data_dir.to_string_lossy(), "/fallback/data/dir");
        assert_eq!(format!("{}", config.node_ip), "4.4.4.4");
        assert_eq!(config.node_labels.len(), 0);
    }

    #[test]
    fn derived_defaults_are_respected() {
        let config_builder = builder_from_json_string(
            r#"{
                "hostname": "k"
        }"#,
        );
        let config = config_builder.unwrap().build(fallbacks()).unwrap();
        assert_eq!(config.node_name, "k");
        assert_eq!(config.hostname, "k");
    }

    #[test]
    fn merging_overrides_all_values() {
        let base_values = builder_from_json_string(
            r#"{
            "listenerPort": 1234,
            "listenerAddress": "172.182.192.1",
            "hostname": "krusty-host",
            "dataDir": "/krusty/data/dir",
            "maxPods": 20,
            "nodeIP": "173.183.193.2",
            "nodeLabels": {
                "label1": "val1",
                "label2": "val2"
            },
            "nodeName": "krusty-node",
            "tlsCertificateFile": "/my/secure/cert.pfx",
            "tlsPrivateKeyFile": "/the/key"
        }"#,
        );
        let override_values = builder_from_json_string(
            r#"{
            "listenerPort": 5678,
            "listenerAddress": "171.181.191.21",
            "hostname": "krusty-host-2",
            "dataDir": "/krusty/data/dir/2",
            "maxPods": 30,
            "nodeIP": "173.183.193.22",
            "nodeLabels": {
                "label21": "val21",
                "label22": "val22"
            },
            "nodeName": "krusty-node-2",
            "tlsCertificateFile": "/my/secure/cert-2.pfx",
            "tlsPrivateKeyFile": "/the/2nd/key"
        }"#,
        );
        let config_builder = base_values.unwrap().with_override(override_values.unwrap());
        let config = config_builder.build(fallbacks()).unwrap();
        assert_eq!(config.server_config.port, 5678);
        assert_eq!(format!("{}", config.server_config.addr), "171.181.191.21");
        assert_eq!(
            config.server_config.tls_cert_file.to_string_lossy(),
            "/my/secure/cert-2.pfx"
        );
        assert_eq!(
            config.server_config.tls_private_key_file.to_string_lossy(),
            "/the/2nd/key"
        );
        assert_eq!(config.node_name, "krusty-node-2");
        assert_eq!(config.hostname, "krusty-host-2");
        assert_eq!(config.max_pods, 30);
        assert_eq!(config.data_dir.to_string_lossy(), "/krusty/data/dir/2");
        assert_eq!(format!("{}", config.node_ip), "173.183.193.22");
        assert_eq!(config.node_labels.len(), 2);
        assert_eq!(
            config.node_labels.get("label21"),
            Some(&("val21".to_owned()))
        );
    }

    #[test]
    fn merging_respects_non_overridden_values() {
        let base_values = builder_from_json_string(
            r#"{
            "listenerPort": 1234,
            "listenerAddress": "172.182.192.1",
            "hostname": "krusty-host",
            "dataDir": "/krusty/data/dir",
            "nodeIP": "173.183.193.2",
            "nodeLabels": {
                "label1": "val1",
                "label2": "val2"
            },
            "nodeName": "krusty-node",
            "tlsCertificateFile": "/my/secure/cert.pfx",
            "tlsPrivateKeyFile": "/the/key"
        }"#,
        );
        let override_values = builder_from_json_string(
            r#"{
            "listenerPort": 2345,
            "nodeName": "krusterrific-node",
            "tlsPrivateKeyFile": "/the/other/key"
        }"#,
        );
        let config_builder = base_values.unwrap().with_override(override_values.unwrap());
        let config = config_builder.build(fallbacks()).unwrap();
        assert_eq!(config.server_config.port, 2345);
        assert_eq!(format!("{}", config.server_config.addr), "172.182.192.1");
        assert_eq!(
            config.server_config.tls_cert_file.to_string_lossy(),
            "/my/secure/cert.pfx"
        );
        assert_eq!(
            config.server_config.tls_private_key_file.to_string_lossy(),
            "/the/other/key"
        );
        assert_eq!(config.node_name, "krusterrific-node");
        assert_eq!(config.hostname, "krusty-host");
        assert_eq!(config.data_dir.to_string_lossy(), "/krusty/data/dir");
        assert_eq!(format!("{}", config.node_ip), "173.183.193.2");
        assert_eq!(config.node_labels.len(), 2);
        assert_eq!(config.node_labels.get("label1"), Some(&("val1".to_owned())));
    }

    #[test]
    fn malformed_config_file_is_reported() {
        let config_builder = builder_from_json_string(
            r#"{
            "listenerPort": 2345,
            "listenerAddress": "173.183.193.2",
            "nodeName": "krustsome-node",
        }"#,
        );
        let error =
            config_builder.expect_err("Expected malformed config to produce error but was okay");
        assert!(
            error.to_string().contains("comma"),
            "Expected malformed config descriptive error"
        );
    }

    #[test]
    fn malformed_config_value_is_reported() {
        let config_builder = builder_from_json_string(
            r#"{
            "listenerPort": "qqqqqqqqqqq",
            "listenerAddress": "173.183.193.2",
            "nodeName": "krustsome-node"
        }"#,
        );
        let error = config_builder
            .unwrap()
            .build(fallbacks())
            .expect_err("Expected config error but was okay");
        assert!(
            error.to_string().contains("invalid type"),
            format!("Expected 'invalid type' but got '{}'", error.to_string())
        );
    }

    #[test]
    fn malformed_config_value_says_which_value() {
        let config_builder = builder_from_json_string(
            r#"{
            "listenerPort": "qqqqqqqqqqq",
            "listenerAddress": "173.183.193.2",
            "nodeName": "krustsome-node"
        }"#,
        );
        let error = config_builder
            .unwrap()
            .build(fallbacks())
            .expect_err("Expected config error but was okay");
        assert!(error.to_string().contains("server port"), error.to_string());
    }

    #[test]
    fn out_of_range_config_value_is_reported() {
        let config_builder = builder_from_json_string(
            r#"{
            "listenerPort": 8675309,
            "listenerAddress": "173.183.193.2",
            "nodeName": "krustsome-node"
        }"#,
        );
        let error = config_builder
            .unwrap()
            .build(fallbacks())
            .expect_err("Expected config error but was okay");
        assert!(
            error.to_string().contains("invalid value"),
            format!("Expected 'invalid value' but got '{}'", error.to_string())
        );
    }

    #[test]
    fn if_invalid_config_value_is_overridden_by_valid_one_it_is_not_an_error() {
        let config_builder_1 = builder_from_json_string(
            r#"{
            "listenerPort": 8675309
        }"#,
        )
        .unwrap();
        let config_builder_2 = builder_from_json_string(
            r#"{
            "listenerPort": 1234
        }"#,
        )
        .unwrap();
        let config_builder = config_builder_1.with_override(config_builder_2);
        let config = config_builder.build(fallbacks());
        assert!(
            config.is_ok(),
            format!("Merged config had error {}", config.unwrap_err())
        );
        assert_eq!(config.unwrap().server_config.port, 1234);
    }

    #[test]
    fn if_invalid_config_value_is_not_overridden_it_is_still_an_error() {
        let config_builder_1 = builder_from_json_string(
            r#"{
            "listenerPort": "qqqqqqqq"
        }"#,
        )
        .unwrap();
        let config_builder_2 = builder_from_json_string(
            r#"{
            "nodeName": "krustsome-node"
        }"#,
        )
        .unwrap();
        let config_builder = config_builder_1.with_override(config_builder_2);
        let error = config_builder
            .build(fallbacks())
            .expect_err("Expected config error but was okay");
        assert!(
            error.to_string().contains("invalid type"),
            format!("Expected 'invalid type' but got '{}'", error.to_string())
        );
    }
}
