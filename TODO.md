# TODO

Connection direction is from "phone" to outside world, outbound is considered Asterisk -> bridge -> Telegram, inbound is the other way around.

## General



## Asterisk Crate

[X] start session with config
[X] register application on startup
[X] manage stasis start -> control has been handed to client
[] Handle requesting a call
[X] add ExternalMedia to capture/send audio packages
[] correctly close audiosockets on hangup
  [] hangup from local (harness/bridge)
  [] hangup from remote (telegram)
[] use ip of process machine for externalMedia (maybe env var)
[] add stability to call if getting error from channel creation (not in stasisApp)

### Asterisk Harness

[] ControlPlane loop -
  [] list applications, client info, etc.
  [] maybe - register different actions
[] understand what _has_ to be on the callback, what can be devided into different parts.
[] close application (hangup on shutdown) gracefully.
[] obtain audio in/out channels
[]

## Telegram Crate

[X] create & persist session
  [x] login
  [x] login with 2FA
  [] logout - maybe
  [] verify logged in session matches phone num
[x] build the listen to updates loop
[x] send messages/signals
  [x] request a call
  [] receive a call
  [] decline a call
  [] end a call
[x] development harness binary to mirror bridge like behavior but have direct
   way to send messages into channels
[x] get all control and audio channels from telegram Handle
## Bridge binary

[] CLI command to login and create a session (a one tiime code is needed)
  [] Main loops should exit cleanly if session is not authed, should direct user to use the login CLI command and restart the app
[] Health checks
[] coordinate shutdown across crates


## Config

* consider builder pattern for configurations

## Long term TODO
