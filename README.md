### Asterisk Configuration

We use a simple prefix scheme to call our stasis application:

```
[from-internal-custom]
exten => _5.,1,NoOp(Telegram bridge user ${EXTEN:1})
 same => n,Stasis(telegram_bridge,${EXTEN:1})
 same => n,Hangup()
```

this allows the IP phone user to dial 5xxx where xxx is the dial number mapped in the router.
