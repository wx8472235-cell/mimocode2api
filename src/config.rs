use std::path::PathBuf;

pub struct Config {
    pub port: u16,
    pub client_file: PathBuf,
    pub default_model: String,
}

impl Config {
    pub fn from_env() -> Self {
        let port = std::env::var("MIMOCODE_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        let client_file = std::env::var("MIMOCODE_CLIENT_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./.mimocode/client"));
        let default_model =
            std::env::var("MIMOCODE_MODEL").unwrap_or_else(|_| "mimo-auto".to_string());
        Self {
            port,
            client_file,
            default_model,
        }
    }
}
