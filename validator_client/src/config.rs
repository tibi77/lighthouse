use crate::graffiti_file::GraffitiFile;
use crate::{http_api, http_metrics};
use clap::ArgMatches;
use clap_utils::{flags::DISABLE_MALLOC_TUNING_FLAG, parse_optional, parse_required};
use directory::{
    get_network_dir, DEFAULT_HARDCODED_NETWORK, DEFAULT_ROOT_DIR, DEFAULT_SECRET_DIR,
    DEFAULT_VALIDATOR_DIR,
};
use eth2::types::Graffiti;
use sensitive_url::SensitiveUrl;
use serde_derive::{Deserialize, Serialize};
use slog::{info, warn, Logger};
use std::fs;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;
use types::{Address, GRAFFITI_BYTES_LEN};

pub const DEFAULT_BEACON_NODE: &str = "http://localhost:5052/";

/// Stores the core configuration for this validator instance.
#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    /// The data directory, which stores all validator databases
    pub validator_dir: PathBuf,
    /// The directory containing the passwords to unlock validator keystores.
    pub secrets_dir: PathBuf,
    /// The http endpoints of the beacon node APIs.
    ///
    /// Should be similar to `["http://localhost:8080"]`
    pub beacon_nodes: Vec<SensitiveUrl>,
    /// An optional beacon node used for block proposals only.
    pub proposer_nodes: Vec<SensitiveUrl>,
    /// If true, the validator client will still poll for duties and produce blocks even if the
    /// beacon node is not synced at startup.
    pub allow_unsynced_beacon_node: bool,
    /// If true, don't scan the validators dir for new keystores.
    pub disable_auto_discover: bool,
    /// If true, re-register existing validators in definitions.yml for slashing protection.
    pub init_slashing_protection: bool,
    /// If true, use longer timeouts for requests made to the beacon node.
    pub use_long_timeouts: bool,
    /// Graffiti to be inserted everytime we create a block.
    pub graffiti: Option<Graffiti>,
    /// Graffiti file to load per validator graffitis.
    pub graffiti_file: Option<GraffitiFile>,
    /// Fallback fallback address.
    pub fee_recipient: Option<Address>,
    /// Configuration for the HTTP REST API.
    pub http_api: http_api::Config,
    /// Configuration for the HTTP REST API.
    pub http_metrics: http_metrics::Config,
    /// Configuration for sending metrics to a remote explorer endpoint.
    pub monitoring_api: Option<monitoring_api::Config>,
    /// If true, enable functionality that monitors the network for attestations or proposals from
    /// any of the validators managed by this client before starting up.
    pub enable_doppelganger_protection: bool,
    /// If true, then we publish validator specific metrics (e.g next attestation duty slot)
    /// for all our managed validators.
    /// Note: We publish validator specific metrics for low validator counts without this flag
    /// (<= 64 validators)
    pub enable_high_validator_count_metrics: bool,
    /// Enable use of the blinded block endpoints during proposals.
    pub builder_proposals: bool,
    /// Overrides the timestamp field in builder api ValidatorRegistrationV1
    pub builder_registration_timestamp_override: Option<u64>,
    /// Fallback gas limit.
    pub gas_limit: Option<u64>,
    /// A list of custom certificates that the validator client will additionally use when
    /// connecting to a beacon node over SSL/TLS.
    pub beacon_nodes_tls_certs: Option<Vec<PathBuf>>,
    /// Delay from the start of the slot to wait before publishing a block.
    ///
    /// This is *not* recommended in prod and should only be used for testing.
    pub block_delay: Option<Duration>,
    /// Disables publishing http api requests to all beacon nodes for select api calls.
    pub disable_run_on_all: bool,
    /// Enables a service which attempts to measure latency between the VC and BNs.
    pub enable_latency_measurement_service: bool,
    /// Defines the number of validators per `validator/register_validator` request sent to the BN.
    pub validator_registration_batch_size: usize,
}

impl Default for Config {
    /// Build a new configuration from defaults.
    fn default() -> Self {
        // WARNING: these directory defaults should be always overwritten with parameters from cli
        // for specific networks.
        let base_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(DEFAULT_ROOT_DIR)
            .join(DEFAULT_HARDCODED_NETWORK);
        let validator_dir = base_dir.join(DEFAULT_VALIDATOR_DIR);
        let secrets_dir = base_dir.join(DEFAULT_SECRET_DIR);

        let beacon_nodes = vec![SensitiveUrl::parse(DEFAULT_BEACON_NODE)
            .expect("beacon_nodes must always be a valid url.")];
        Self {
            validator_dir,
            secrets_dir,
            beacon_nodes,
            proposer_nodes: Vec::new(),
            allow_unsynced_beacon_node: false,
            disable_auto_discover: false,
            init_slashing_protection: false,
            use_long_timeouts: false,
            graffiti: None,
            graffiti_file: None,
            fee_recipient: None,
            http_api: <_>::default(),
            http_metrics: <_>::default(),
            monitoring_api: None,
            enable_doppelganger_protection: false,
            enable_high_validator_count_metrics: false,
            beacon_nodes_tls_certs: None,
            block_delay: None,
            builder_proposals: false,
            builder_registration_timestamp_override: None,
            gas_limit: None,
            disable_run_on_all: false,
            enable_latency_measurement_service: true,
            validator_registration_batch_size: 500,
        }
    }
}

impl Config {
    /// Returns a `Default` implementation of `Self` with some parameters modified by the supplied
    /// `cli_args`.
    pub fn from_cli(cli_args: &ArgMatches, log: &Logger) -> Result<Config, String> {
        let mut config = Config::default();

        let default_root_dir = dirs::home_dir()
            .map(|home| home.join(DEFAULT_ROOT_DIR))
            .unwrap_or_else(|| PathBuf::from("."));

        let (mut validator_dir, mut secrets_dir) = (None, None);
        if cli_args.value_of("datadir").is_some() {
            let base_dir: PathBuf = parse_required(cli_args, "datadir")?;
            validator_dir = Some(base_dir.join(DEFAULT_VALIDATOR_DIR));
            secrets_dir = Some(base_dir.join(DEFAULT_SECRET_DIR));
        }
        if cli_args.value_of("validators-dir").is_some() {
            validator_dir = Some(parse_required(cli_args, "validators-dir")?);
        }
        if cli_args.value_of("secrets-dir").is_some() {
            secrets_dir = Some(parse_required(cli_args, "secrets-dir")?);
        }

        config.validator_dir = validator_dir.unwrap_or_else(|| {
            default_root_dir
                .join(get_network_dir(cli_args))
                .join(DEFAULT_VALIDATOR_DIR)
        });

        config.secrets_dir = secrets_dir.unwrap_or_else(|| {
            default_root_dir
                .join(get_network_dir(cli_args))
                .join(DEFAULT_SECRET_DIR)
        });

        if !config.validator_dir.exists() {
            fs::create_dir_all(&config.validator_dir)
                .map_err(|e| format!("Failed to create {:?}: {:?}", config.validator_dir, e))?;
        }

        if let Some(beacon_nodes) = parse_optional::<String>(cli_args, "beacon-nodes")? {
            config.beacon_nodes = beacon_nodes
                .split(',')
                .map(SensitiveUrl::parse)
                .collect::<Result<_, _>>()
                .map_err(|e| format!("Unable to parse beacon node URL: {:?}", e))?;
        }
        // To be deprecated.
        else if let Some(beacon_node) = parse_optional::<String>(cli_args, "beacon-node")? {
            warn!(
                log,
                "The --beacon-node flag is deprecated";
                "msg" => "please use --beacon-nodes instead"
            );
            config.beacon_nodes = vec![SensitiveUrl::parse(&beacon_node)
                .map_err(|e| format!("Unable to parse beacon node URL: {:?}", e))?];
        }
        // To be deprecated.
        else if let Some(server) = parse_optional::<String>(cli_args, "server")? {
            warn!(
                log,
                "The --server flag is deprecated";
                "msg" => "please use --beacon-nodes instead"
            );
            config.beacon_nodes = vec![SensitiveUrl::parse(&server)
                .map_err(|e| format!("Unable to parse beacon node URL: {:?}", e))?];
        }

        if let Some(proposer_nodes) = parse_optional::<String>(cli_args, "proposer_nodes")? {
            config.proposer_nodes = proposer_nodes
                .split(',')
                .map(SensitiveUrl::parse)
                .collect::<Result<_, _>>()
                .map_err(|e| format!("Unable to parse proposer node URL: {:?}", e))?;
        }

        if cli_args.is_present("delete-lockfiles") {
            warn!(
                log,
                "The --delete-lockfiles flag is deprecated";
                "msg" => "it is no longer necessary, and no longer has any effect",
            );
        }

        if cli_args.is_present("allow-unsynced") {
            warn!(
                log,
                "The --allow-unsynced flag is deprecated";
                "msg" => "it no longer has any effect",
            );
        }
        config.disable_run_on_all = cli_args.is_present("disable-run-on-all");
        config.disable_auto_discover = cli_args.is_present("disable-auto-discover");
        config.init_slashing_protection = cli_args.is_present("init-slashing-protection");
        config.use_long_timeouts = cli_args.is_present("use-long-timeouts");

        if let Some(graffiti_file_path) = cli_args.value_of("graffiti-file") {
            let mut graffiti_file = GraffitiFile::new(graffiti_file_path.into());
            graffiti_file
                .read_graffiti_file()
                .map_err(|e| format!("Error reading graffiti file: {:?}", e))?;
            config.graffiti_file = Some(graffiti_file);
            info!(log, "Successfully loaded graffiti file"; "path" => graffiti_file_path);
        }

        if let Some(input_graffiti) = cli_args.value_of("graffiti") {
            let graffiti_bytes = input_graffiti.as_bytes();
            if graffiti_bytes.len() > GRAFFITI_BYTES_LEN {
                return Err(format!(
                    "Your graffiti is too long! {} bytes maximum!",
                    GRAFFITI_BYTES_LEN
                ));
            } else {
                let mut graffiti = [0; 32];

                // Copy the provided bytes over.
                //
                // Panic-free because `graffiti_bytes.len()` <= `GRAFFITI_BYTES_LEN`.
                graffiti[..graffiti_bytes.len()].copy_from_slice(graffiti_bytes);

                config.graffiti = Some(graffiti.into());
            }
        }

        if let Some(input_fee_recipient) =
            parse_optional::<Address>(cli_args, "suggested-fee-recipient")?
        {
            config.fee_recipient = Some(input_fee_recipient);
        }

        if let Some(tls_certs) = parse_optional::<String>(cli_args, "beacon-nodes-tls-certs")? {
            config.beacon_nodes_tls_certs = Some(tls_certs.split(',').map(PathBuf::from).collect());
        }

        /*
         * Http API server
         */

        if cli_args.is_present("http") {
            config.http_api.enabled = true;
        }

        if let Some(address) = cli_args.value_of("http-address") {
            if cli_args.is_present("unencrypted-http-transport") {
                config.http_api.listen_addr = address
                    .parse::<IpAddr>()
                    .map_err(|_| "http-address is not a valid IP address.")?;
            } else {
                return Err(
                    "While using `--http-address`, you must also use `--unencrypted-http-transport`."
                        .to_string(),
                );
            }
        }

        if let Some(port) = cli_args.value_of("http-port") {
            config.http_api.listen_port = port
                .parse::<u16>()
                .map_err(|_| "http-port is not a valid u16.")?;
        }

        if let Some(allow_origin) = cli_args.value_of("http-allow-origin") {
            // Pre-validate the config value to give feedback to the user on node startup, instead of
            // as late as when the first API response is produced.
            hyper::header::HeaderValue::from_str(allow_origin)
                .map_err(|_| "Invalid allow-origin value")?;

            config.http_api.allow_origin = Some(allow_origin.to_string());
        }

        /*
         * Prometheus metrics HTTP server
         */

        if cli_args.is_present("metrics") {
            config.http_metrics.enabled = true;
        }

        if cli_args.is_present("enable-high-validator-count-metrics") {
            config.enable_high_validator_count_metrics = true;
        }

        if let Some(address) = cli_args.value_of("metrics-address") {
            config.http_metrics.listen_addr = address
                .parse::<IpAddr>()
                .map_err(|_| "metrics-address is not a valid IP address.")?;
        }

        if let Some(port) = cli_args.value_of("metrics-port") {
            config.http_metrics.listen_port = port
                .parse::<u16>()
                .map_err(|_| "metrics-port is not a valid u16.")?;
        }

        if let Some(allow_origin) = cli_args.value_of("metrics-allow-origin") {
            // Pre-validate the config value to give feedback to the user on node startup, instead of
            // as late as when the first API response is produced.
            hyper::header::HeaderValue::from_str(allow_origin)
                .map_err(|_| "Invalid allow-origin value")?;

            config.http_metrics.allow_origin = Some(allow_origin.to_string());
        }

        if cli_args.is_present(DISABLE_MALLOC_TUNING_FLAG) {
            config.http_metrics.allocator_metrics_enabled = false;
        }

        /*
         * Explorer metrics
         */
        if let Some(monitoring_endpoint) = cli_args.value_of("monitoring-endpoint") {
            let update_period_secs =
                clap_utils::parse_optional(cli_args, "monitoring-endpoint-period")?;
            config.monitoring_api = Some(monitoring_api::Config {
                db_path: None,
                freezer_db_path: None,
                update_period_secs,
                monitoring_endpoint: monitoring_endpoint.to_string(),
            });
        }

        if cli_args.is_present("enable-doppelganger-protection") {
            config.enable_doppelganger_protection = true;
        }

        if cli_args.is_present("builder-proposals") {
            config.builder_proposals = true;
        }

        config.gas_limit = cli_args
            .value_of("gas-limit")
            .map(|gas_limit| {
                gas_limit
                    .parse::<u64>()
                    .map_err(|_| "gas-limit is not a valid u64.")
            })
            .transpose()?;

        if let Some(registration_timestamp_override) =
            cli_args.value_of("builder-registration-timestamp-override")
        {
            config.builder_registration_timestamp_override = Some(
                registration_timestamp_override
                    .parse::<u64>()
                    .map_err(|_| "builder-registration-timestamp-override is not a valid u64.")?,
            );
        }

        if cli_args.is_present("strict-fee-recipient") {
            warn!(
                log,
                "The flag `--strict-fee-recipient` has been deprecated due to a bug causing \
                missed proposals. The flag will be ignored."
            );
        }

        config.enable_latency_measurement_service =
            parse_optional(cli_args, "latency-measurement-service")?.unwrap_or(true);

        config.validator_registration_batch_size =
            parse_required(cli_args, "validator-registration-batch-size")?;
        if config.validator_registration_batch_size == 0 {
            return Err("validator-registration-batch-size cannot be 0".to_string());
        }

        /*
         * Experimental
         */
        if let Some(delay_ms) = parse_optional::<u64>(cli_args, "block-delay-ms")? {
            config.block_delay = Some(Duration::from_millis(delay_ms));
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Ensures the default config does not panic.
    fn default_config() {
        Config::default();
    }
}
