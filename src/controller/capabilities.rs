#[derive(clap::ValueEnum, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientCapability {
    #[value(name = "readFile")]
    ReadFile,
}

#[derive(Debug, Clone)]
pub struct ClientCapabilities {
    read_file: bool,
}

impl ClientCapabilities {
    pub fn new(client_capabilities: Option<&[ClientCapability]>) -> Self {
        let mut c = ClientCapabilities { read_file: false };
        if let Some(client_capabilities) = client_capabilities {
            for capability in client_capabilities {
                match capability {
                    ClientCapability::ReadFile => c.read_file = true,
                }
            }
        }
        c
    }
    pub fn can_read_file(&self) -> bool {
        self.read_file
    }
}
