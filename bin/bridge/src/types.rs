use asterisk::AsteriskHandler;
use telegram::TelegramHandle;
use tokio::task::JoinHandle;

pub struct ActorTaskHandlers {
    pub asterisk: JoinHandle<anyhow::Result<()>>,
    pub telegram: JoinHandle<()>,
}

pub struct ActorHandlers {
    asterisk: AsteriskHandler,
    telegram: TelegramHandle,
}

impl ActorHandlers {
    pub fn new(asterisk: AsteriskHandler, telegram: TelegramHandle) -> Self {
        Self { asterisk, telegram }
    }
}
