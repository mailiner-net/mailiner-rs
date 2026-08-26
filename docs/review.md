# mailiner-core review — remaining findings

Five independent reviews of `mailiner-core` (types, connector, storage, submit/errors, body/memory) on 2026-08-26. First-wave cleanups and the bugs that did not need a redesign landed in `6a43500`–`b14f132`. This note is the leftover backlog: still true at those commits, with current file locations.

Severity matches the reviews: **bug** is correctness or user-visible breakage, **suggestion** is a real design gap, **nit** is small.

## Already addressed

Do not re-open these.

| Finding | Landed in |
|---|---|
| Unused `Storage` / `InMemoryStorage`, `AccountMetadata`, `FolderMetadata` | `6a43500` |
| Unused `MessagePart.raw_content`, `LoadedMessage.structure`, `anyhow` | `6a43500` |
| `EmailAddress` concatenated recipients with no separator | `e636015` |
| `load_message` treated a decoded cid image as a successful body | `96ee264` |
| `content_parts()` included hidden cid inlines (`should_prefetch`) | `96ee264` |
| BODYSTRUCTURE cache keyed only by UID (Inbox/Sent collision) | `712a476` |
| `get_envelope` always `SELECT`ed INBOX | `712a476` |
| List index not pruned after our own MOVE/DELETE | `712a476` |
| SMTP errors classified from `Display` text | `ccaef37` |
| Unknown persisted `SendErrorKind` had no `#[serde(other)]` | `ccaef37` |
| Send cancel ignored the oneshot; cancelled outbox marked Failed | `ccaef37` |
| SMTP success reply not truncated | `ccaef37` |
| Attachment download hint used decoded `size` instead of wire size | `b14f132` |
| Send cancel requeued before DATA finished (double-send) | `322db54` |
| Date/sequence list index stale after remote EXPUNGE | `289102a` |
| Envelope had no RFC Message-ID / threading headers | `611fa4d` |
| Stringly `update_envelope_flags` | `5e45729` |

---

## Remaining

### 1. `MessageId` is a folder-scoped IMAP UID in a global newtype

- Severity: suggestion (design)
- Where: [`crates/mailiner-core/src/ids.rs`](../crates/mailiner-core/src/ids.rs), [`crates/mailiner-app/src/message.rs`](../crates/mailiner-app/src/message.rs)
- IMAP UIDs are unique per mailbox. Core `MessageId` is an unvalidated `String` (`MessageId::new(uid.to_string())` in the IMAP connector). The same numeric UID in Inbox and Sent is the same id. `::new` also accepts empty strings.
- The app cannot use the core type as-is: it has a second `MessageId` (and `MailboxId`) and round-trips with `to_string()`.
- Adding `folder_id` to `get_envelope` (`712a476`) closed the immediate footgun; every other message API already took a folder. The identity model is still leaky.
- Direction: make `MessageId` folder-scoped (or require `FolderId` on every message op), share one ID type with the app (`From`/`AsRef` without string copies), reject empty IDs.

### 2. List index and Date/sequence paging after a remote EXPUNGE

- [x] Done. Date pages by `UID SEARCH ALL` (newest UID first). Each range `SELECT`s and rebuilds the index when `EXISTS` ≠ cached total. Sequence fetch remains only if SEARCH ALL fails.
- Still open (not required to close this item): IDLE / periodic NOOP so the UI notices EXPUNGE without a later range fetch.

### 3. Envelope has no RFC 5322 identity / threading headers

- [x] Done. `Envelope` now has `reply_to`, `rfc_message_id`, `in_reply_to`, and `references`. IMAP parses them from headers; reply/reply-all prefill sets draft threading and prefers `Reply-To`.

### 4. Flags are stringly typed; star vs flag is unused

- [x] Done. `EnvelopeFlag` is the trait argument; IMAP maps it to atoms. `Starred` (`\Starred`) and `Flagged` (`\Flagged`) stay distinct; the app still does not surface either.

### 5. Unused `EmailConnector` methods

- Severity: suggestion
- Where: [`crates/mailiner-core/src/connector.rs`](../crates/mailiner-core/src/connector.rs)
- `create_folder`, `delete_folder`, `open_folder`, and `list_envelopes` have no callers outside the trait/impls. `get_envelope` is also unused after the folder-id fix.
- `open_folder` is `prepare_folder_list(..., Date)`. `list_envelopes` calls that then fetches `0..total`, so it would wipe an Unread/Size/Sender index if anyone used it. Folder create/delete assume `/` as the hierarchy delimiter.
- Direction: remove or `#[doc(hidden)]` until the UI needs them. If `list_envelopes` stays, it must honor the current sort, not reset to Date.

### 6. `MockConnector` does not implement the contracts

- Severity: suggestion
- Where: [`crates/mailiner-core/src/connector.rs`](../crates/mailiner-core/src/connector.rs) (`MockConnector`)
- `connected` is never written (`allow(dead_code)`). `list_envelopes_range` ignores sort and returns oldest-first (`test-message-1` at index 0) while IMAP Date is newest-first. `prepare_folder_list` always reports `unread: Some(3)` and `supports_size_sender: true`. `sync_unread_sort_index` always returns `[]`, so unread-first relocate tests would pass without moving rows. `move_messages` returns the **source** IDs as destination UIDs, which would make undo look successful. HTML part `1.2` is advertised as quoted-printable but `mock_section_bytes` returns raw HTML.
- The mock’s own unit tests never call the trait methods; `load_message` tests use it as a fixture.
- Direction: make the mock stateful (sort, flags, dest UIDs, connection) or name it a UI fixture and keep contract tests on IMAP / `sort.rs`. Do not return source IDs from `move_messages`.

### 7. Prefetch has no size bound; viewer holds wire + decoded + data-URL copies

- Severity: suggestion (WASM memory)
- Where: [`crates/mailiner-app/src/message_loader.rs`](../crates/mailiner-app/src/message_loader.rs), [`crates/mailiner-core/src/connector.rs`](../crates/mailiner-core/src/connector.rs) (`fetch_raw_parts`), [`crates/mailiner-app/src/formatter/html.rs`](../crates/mailiner-app/src/formatter/html.rs)
- `load_message` still prefetches every `should_prefetch` part (visible body **and** hidden cid images) via a complete `HashMap<String, Vec<u8>>`. There is no `original_size` gate (unlike `stream_raw_part` / 100 MiB). MIME decode caps run only after the wire `Vec`s exist. `format_html` then base64-encodes the same `Binary` buffers into data URLs while the parts still hold the decoded bytes. Peak usage is wire + decoded + data-URL HTML.
- `FormatResult.inlined_part_ids` is computed but never used to hide or free those parts.
- Direction: bound prefetch by `original_size`; stream or skip large cid images; drop `MessageContent::Binary` after inlining (or inline from the stream). Keep `fetch_raw_parts` for small text.

### 8. `fetch_raw_parts` swallows missing sections

- [x] Done. A missing requested section is now an error; the loader already fails if no display part decoded.

### 9. MOVE fallback can copy without deleting

- [x] Done. COPY-then-delete failure is `MailinerError::PartialMove`. The UI keeps the source rows and tells the user not to retry.

### 10. `folder_counts` treats missing `UNSEEN` as zero

- [x] Done. STATUS without `UNSEEN` is skipped (same as a failed STATUS); the sidebar keeps its last badge instead of showing a false zero.

### 11. Send cancel can still double-send

- [x] Done. Cancel keeps the in-flight slot (`Sending`) until `SmtpFinished`. Success deletes the row even if `inflight` was already dropped; `Cancelled` is what requeues.

### 12. `MailinerError` is stringly typed; connect classification re-parses English

- Severity: suggestion
- Where: [`crates/mailiner-core/src/error.rs`](../crates/mailiner-core/src/error.rs), [`crates/mailiner-app/src/connection.rs`](../crates/mailiner-app/src/connection.rs) (`classify_mailiner_error`)
- IMAP maps into `Connector(String)`. `classify_mailiner_error` scans `Display` for `"auth"` / `"password"` / `"tls"`. That loses type information and can mis-classify. The send path correctly uses `SendErrorKind` instead.
- Direction: structured IMAP/connect variants (or source-carrying wrappers) so classification does not depend on English substrings. Keep send errors on `SendErrorKind`.

### 13. `EmailConnector` is parameterized on the stream type

- Severity: suggestion
- Where: [`crates/mailiner-core/src/connector.rs`](../crates/mailiner-core/src/connector.rs)
- The whole trait is generic on `S` plus `async_trait`’s default `Send` futures. The live app stores a concrete `ImapConnector<WebSocketStream>`, so this is not what blocks WASM today, but `load_message` still needs a dummy `NullStream`, and you cannot hold a transport-erased connector after `connect`. `S` is only consumed by `connect`.
- Direction: associated type, or split `connect(stream)` from a non-generic session trait used by list/fetch/stream.

### 14. `MessageSort::Date` is arrival, not the Date header

- [x] Done. Variant is `Arrival` (label “Arrival”). Persisted `"date"` still loads; serde has `alias = "date"`.

### 15. `PartKind::Other` is never produced

- [x] Done. Removed. Unknown leaves stay `Attachment`; the formatter only treats `TextPlain` as plain text.

### 16. Leftover `Account` / timestamps / `TlsModeUnsupported`

- Severity: nit
- `authenticate` returns `mailiner_core::Account` and the app discards it (`AccountConfig` is the real record). `created_at` / `updated_at` on `Account` / `Folder` / `Envelope` are set to `Utc::now()` on every fetch and nobody reads them.
- `SendErrorKind::TlsModeUnsupported` is unused in preflight (every `SmtpTlsMode` is spoken) but still has UI copy in `send.rs`.
- Direction: drop `Account` from the connector return or align it with `AccountConfig`. Delete `TlsModeUnsupported` or keep it only as a real future variant.

---

## Suggested order

1. Folder-scoped `MessageId` (1) — unblock a lot of later IMAP work; do not mix with a feature PR.
3. Prefetch bounds (7) — WASM memory and load honesty.
4. Slim the connector trait (5, 6, 13) and leftover types (12, 16) when touching those files anyway.
