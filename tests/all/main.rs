#[path = "../common/mod.rs"]
mod common;
#[path = "../support/http_server.rs"]
mod http_server;

mod board;
mod cli;
mod config_edit;
mod config_reload;
mod flow;
mod issues;
mod live_github;
mod mcp_http;
mod memory;
mod merge;
mod pools;
mod reconcile;
mod routing;
mod sandbox;
mod settings;
mod setup;
mod setup_steps;
mod skills_install;
mod slots;
mod t3_http;
mod types;
mod update;
mod upstream_catalog;
mod upstream_client;
mod upstream_e2e;
mod upstream_manager;
mod wf;
