from nostr_sdk import Client, NostrSigner, Keys, Event, UnsignedEvent, EventBuilder, Filter, \
    HandleNotification, Timestamp, nip04_decrypt, nip59_extract_rumor, SecretKey, init_logger, LogLevel, Kind, KindEnum
import time

init_logger(LogLevel.DEBUG)

# sk = SecretKey.from_bech32("nsec1ufnus6pju578ste3v90xd5m2decpuzpql2295m3sknqcjzyys9ls0qlc85")
# keys = Keys(sk)
# OR
keys = Keys.parse("nsec1ufnus6pju578ste3v90xd5m2decpuzpql2295m3sknqcjzyys9ls0qlc85")

sk = keys.secret_key()
pk = keys.public_key()
print(f"Bot public key: {pk.to_bech32()}")

signer = NostrSigner.keys(keys)
client = Client(signer)

client.add_relay("wss://relay.damus.io")
client.add_relay("wss://nostr.mom")
client.add_relay("wss://nostr.oxtr.dev")
client.connect()

nip04_filter = Filter().pubkey(pk).kind(Kind.from_enum(KindEnum.ENCRYPTED_DIRECT_MESSAGE())).since(Timestamp.now())
nip59_filter = Filter().pubkey(pk).kind(Kind.from_enum(KindEnum.GIFT_WRAP())).since(Timestamp.from_secs(Timestamp.now().as_secs() - 60 * 60 * 24 * 7)) # NIP59 have a tweaked timestamp (in the past)
client.subscribe([nip04_filter, nip59_filter], None)

class NotificationHandler(HandleNotification):
    def handle(self, relay_url, subscription_id, event: Event):
        print(f"Received new event from {relay_url}: {event.as_json()}")
        if event.kind().match_enum(KindEnum.ENCRYPTED_DIRECT_MESSAGE()):
            print("Decrypting NIP04 event")
            try:
                msg = nip04_decrypt(sk, event.author(), event.content())
                print(f"Received new msg: {msg}")
                client.send_direct_msg(event.author(), f"Echo: {msg}", event.id())
            except Exception as e:
                print(f"Error during content NIP04 decryption: {e}")
        elif event.kind().match_enum(KindEnum.GIFT_WRAP()):
            print("Decrypting NIP59 event")
            try:
                rumor: UnsignedEvent = nip59_extract_rumor(keys, event)
                if rumor.kind().match_enum(KindEnum.SEALED_DIRECT()):
                    msg = rumor.content()
                    print(f"Received new msg [sealed]: {msg}")
                    client.send_sealed_msg(rumor.author(), f"Echo: {msg}", None)
                else:
                    print(f"{rumor.as_json()}")
            except Exception as e:
                print(f"Error during content NIP59 decryption: {e}")

    def handle_msg(self, relay_url, msg):
        None
    
client.handle_notifications(NotificationHandler())

while True:
    time.sleep(5.0)