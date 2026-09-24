use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cucumber::World;
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_adapters::{build_http_test_stack, BusStackRuntime, DaliRuntimeConfig, StaticAsset};
use dali2rust_bus::BusConfig;
use dali2rust_platform::slice_store::SliceStore;
use dali2rust_bsp::slice_store_files::InMemorySliceStore;
use dali2rust_test_support::wait_for_tcp_ready;
use dali2rust_adapters::http::host::HostServer;

pub mod steps;
pub mod ws_client;

pub const WEB_FIXTURE_INDEX_HTML: &[u8] = include_bytes!("../fixtures/web/index.html.gz");
pub const WEB_FIXTURE_APP_JS: &[u8] = include_bytes!("../fixtures/web/app.js.gz");

const WEB_TEST_ASSETS: &[StaticAsset] = &[
    StaticAsset {
        route_path: "/",
        content_type: "text/html",
        gzip_body: WEB_FIXTURE_INDEX_HTML,
    },
    StaticAsset {
        route_path: "/assets/app.js",
        content_type: "application/javascript",
        gzip_body: WEB_FIXTURE_APP_JS,
    },
];

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct DaliWorld {
    server_port: u16,
    stop: Arc<AtomicBool>,
    server_thread: Option<thread::JoinHandle<()>>,
    last_response: Option<TestResponse>,
    dali_mock: Arc<Mutex<MockDaliTransport>>,
    mock_unblock_flag: Arc<AtomicBool>,
    mqtt_mock: Arc<dali2rust_mqtt_runtime::MockMqttClient>,
    _runtime: Option<Box<BusStackRuntime>>,
    persistence_slices: Option<Arc<InMemorySliceStore>>,
    runtime_config: DaliRuntimeConfig,
    pub stored_responses: Vec<TestResponse>,
    pub remembered_u64: Option<u64>,
    pub remembered_json: Option<serde_json::Value>,
    pub remembered_flag: Option<bool>,
    pub firmware_image_url: Option<String>,
    pub remembered_pair: Option<(u64, u64)>,
    pub background_handle: Option<thread::JoinHandle<Option<(TestResponse, Duration)>>>,
    pub ws_clients: Vec<ws_client::WsTestClient>,
    pub kept_body: Vec<u8>,
    last_request_elapsed: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct TestResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: String,
    pub content_encoding: String,
    pub headers: Vec<(String, String)>,
}

impl DaliWorld {
    fn new() -> Self {
        let dali_mock = Arc::new(Mutex::new(MockDaliTransport::new()));
        let mock_unblock_flag = dali_mock.lock().unwrap().get_unblock_flag();
        let mut world = Self {
            server_port: 0,
            stop: Arc::new(AtomicBool::new(false)),
            server_thread: None,
            last_response: None,
            last_request_elapsed: None,
            dali_mock,
            mock_unblock_flag,
            mqtt_mock: dali2rust_mqtt_runtime::MockMqttClient::bundle().0,
            _runtime: None,
            persistence_slices: None,
            runtime_config: DaliRuntimeConfig::default(),
            stored_responses: Vec::new(),
            remembered_u64: None,
            remembered_json: None,
            remembered_flag: None,
            firmware_image_url: None,
            remembered_pair: None,
            background_handle: None,
            ws_clients: Vec::new(),
            kept_body: Vec::new(),
        };
        world.start_test_server();
        world
    }

    fn start_test_server(&mut self) {
        self.launch_server(BusConfig::default(), self.runtime_config);
    }

    fn launch_server(&mut self, bus_config: BusConfig, runtime_config: DaliRuntimeConfig) {
        let server = HostServer::bind("127.0.0.1:0").expect("bind test server");
        self.server_port = server.port();

        let stop = self.stop.clone();
        let (mqtt_mock, mqtt_bundle) = dali2rust_mqtt_runtime::MockMqttClient::bundle();
        self.mqtt_mock = mqtt_mock;
        let (router, ws_hub, runtime) = build_http_test_stack(
            "0.1.0-test",
            self.dali_mock.clone(),
            bus_config,
            runtime_config,
            self.persistence_slices_api(),
            WEB_TEST_ASSETS,
            Some(mqtt_bundle),
        );
        self._runtime = Some(runtime);
        let router = Arc::new(router);

        self.server_thread = Some(thread::spawn(move || {
            let _ = dali2rust_adapters::http::host::run_blocking(&server, router, ws_hub, &stop);
        }));

        wait_for_tcp_ready(self.server_port, Duration::from_secs(2));
    }

    fn persistence_slices_api(&self) -> Option<Arc<dyn SliceStore>> {
        self.persistence_slices
            .as_ref()
            .map(|fs| Arc::clone(fs) as Arc<dyn SliceStore>)
    }

    fn reset_mock_transport(&mut self) {
        let mock = MockDaliTransport::new();
        self.mock_unblock_flag = mock.get_unblock_flag();
        self.dali_mock = Arc::new(Mutex::new(mock));
    }

    pub fn restart_server_with_config(&mut self, bus_config: BusConfig) {
        self.restart_server_with_configs(bus_config, self.runtime_config);
    }

    pub fn restart_server_with_configs(
        &mut self,
        bus_config: BusConfig,
        runtime_config: DaliRuntimeConfig,
    ) {
        for client in &mut self.ws_clients {
            client.close();
        }
        self.ws_clients.clear();
        self.mock_unblock_flag.store(false, Ordering::Release);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.server_thread.take() {
            let _ = h.join();
        }

        self.stop = Arc::new(AtomicBool::new(false));
        self._runtime = None;
        self.runtime_config = runtime_config;
        self.reset_mock_transport();
        self.launch_server(bus_config, runtime_config);
    }

    pub fn restart_server(&mut self) {
        self.restart_server_with_config(BusConfig::default());
    }

    pub fn enable_in_memory_persistence(&mut self) {
        if self.persistence_slices.is_none() {
            self.persistence_slices = Some(Arc::new(InMemorySliceStore::new()));
        }
    }


    pub fn send_http_request_raw(
        port: u16,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: &str,
    ) -> TestResponse {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("tcp connect");
        let body_content = body.map(|b| b.to_vec()).unwrap_or_default();
        let content_length = body_content.len();

        let req = if content_length > 0 {
            format!(
                "{method} {path} HTTP/1.1\r\n\
                 Host: 127.0.0.1\r\n\
                 Content-Type: {content_type}\r\n\
                 Content-Length: {content_length}\r\n\
                 Connection: close\r\n\r\n"
            )
        } else {
            format!(
                "{method} {path} HTTP/1.1\r\n\
                 Host: 127.0.0.1\r\n\
                 Connection: close\r\n\r\n"
            )
        };

        stream.write_all(req.as_bytes()).expect("write request");
        if content_length > 0 {
            stream.write_all(&body_content).expect("write body");
        }

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).expect("read response");
        let s = String::from_utf8_lossy(&buf);

        let sep = s
            .find("\r\n\r\n")
            .expect("response should have header/body separator");
        let header_part = &s[..sep];
        let body_part = &buf[sep + 4..];

        let status: u16 = header_part
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let header_value = |name: &str| {
            header_part
                .lines()
                .find(|l| l.to_lowercase().starts_with(name))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
                .unwrap_or_default()
        };
        let content_type_resp = header_value("content-type:");
        let content_encoding_resp = header_value("content-encoding:");

        let headers = header_part
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_string()))
            .collect();
        TestResponse {
            status,
            body: body_part.to_vec(),
            content_type: content_type_resp,
            content_encoding: content_encoding_resp,
            headers,
        }
    }

    pub fn send_http_request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: &str,
    ) {
        let started = Instant::now();
        let response =
            Self::send_http_request_raw(self.server_port, method, path, body, content_type);
        self.last_request_elapsed = Some(started.elapsed());
        self.last_response = Some(response);
    }

    pub fn send_http_request_background(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: &str,
    ) {
        let port = self.server_port;
        let method = method.to_string();
        let path = path.to_string();
        let body = body.map(|b| b.to_vec());
        let content_type = content_type.to_string();

        if let Some(h) = self.background_handle.take() {
            let _ = h.join();
        }
        self.background_handle = Some(thread::spawn(move || {
            let started = Instant::now();
            let response =
                Self::send_http_request_raw(port, &method, &path, body.as_deref(), &content_type);
            Some((response, started.elapsed()))
        }));
    }

    pub fn drain_background(&mut self) {
        if let Some(h) = self.background_handle.take() {
            if let Ok(Some((response, elapsed))) = h.join() {
                self.last_request_elapsed = Some(elapsed);
                self.last_response = Some(response);
            }
        }
    }

    pub fn last_response(&self) -> Option<&TestResponse> {
        self.last_response.as_ref()
    }

    pub fn last_request_elapsed(&self) -> Option<Duration> {
        self.last_request_elapsed
    }

    pub fn dali_mock(&self) -> &Arc<Mutex<MockDaliTransport>> {
        &self.dali_mock
    }

    pub fn mqtt_mock(&self) -> &Arc<dali2rust_mqtt_runtime::MockMqttClient> {
        &self.mqtt_mock
    }

    pub fn server_port(&self) -> u16 {
        self.server_port
    }
}

impl Drop for DaliWorld {
    fn drop(&mut self) {
        for client in &mut self.ws_clients {
            client.close();
        }
        self.ws_clients.clear();
        self.mock_unblock_flag.store(false, Ordering::Release);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.server_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.background_handle.take() {
            let _ = h.join();
        }
    }
}

fn main() {
    let features_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("features");
    futures::executor::block_on(
        DaliWorld::cucumber()
            .max_concurrent_scenarios(1)
            .fail_on_skipped()
            .filter_run_and_exit(features_dir, |feature, _, scenario| {
                let has_wip = feature.tags.iter().any(|t| t == "wip")
                    || scenario.tags.iter().any(|t| t == "wip");
                let only_features = std::env::var("ONLY_FEATURES").ok();
                let features_ok = only_features.as_ref().map_or(true, |pat| {
                    feature
                        .path
                        .as_ref()
                        .is_some_and(|p| p.to_string_lossy().contains(pat.as_str()))
                });
                !has_wip && features_ok
            }),
    );
}
