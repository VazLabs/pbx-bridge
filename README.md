### Asterisk Configuration

We use a simple prefix scheme to call our stasis application:

```
[from-internal-custom]
exten => _5.,1,NoOp(Telegram bridge user ${EXTEN:1})
 same => n,Stasis(telegram_bridge,${EXTEN:1})
 same => n,Hangup()
```

this allows the IP phone user to dial 5xxx where xxx is the dial number mapped in the router.

## Running on Docker

Due to how Docker bind and mounts files vs directories, to avoid session persistence issues, it is recommended to bind an entire directory for session data, and keep the session file inside it.
Use the session_file field in the `telegram` configuraiton to point to the file in the directory.

### Example

With the file in `./session-data/myuser.session`


```toml
# bridge_config.toml
[telegram]
session_file = "session-data/myuser.session"
# ... other fields

```

then run docker like so, adjusting other values as needed:

```bash
docker run --rm -it \
-v "$(pwd)/bridge_config.toml:/app/bridge_config.toml" \
-v "$(pwd)/session-data:/app/session-data" \
-p 40000:40000/tcp \
-e RUST_BACKTRACE=1 -e RUST_LOG=info \
pbx-bridge:new-telegram ./target/debug/telegram_harness
```
