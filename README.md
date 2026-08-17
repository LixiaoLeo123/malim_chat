# malim_chat

`malim_chat` is a server-authoritative AI chat application with a Tauri 2 + React client and a Rust API service.

## Architecture

- The Rust server and PostgreSQL are the source of truth for accounts, conversations, messages, settings, provider keys, and sync cursors.
- Clients keep only a bounded encrypted cache and an idempotent mutation outbox. They render local sends immediately, then reconcile against server-assigned sequence numbers.
- Conversation lists use keyset pagination. Message histories load backward in pages and never fetch an entire long conversation to render it.
- Provider credentials are AES-256-GCM encrypted at rest on the server. The encryption key is supplied through `MALIM_ENCRYPTION_KEY` and is never sent to a client.
- Every message supports selected-word local dictionary lookup: Russian-to-English (OpenRussian), German-to-English (Kaikki), and English-to-Chinese (ECDICT). Results include rich definitions and, where present, forms, frequency, tags, and lexical metadata.

## Local development

1. Copy `apps/server/.env.example` to `apps/server/.env` and set secure development values.
2. Start PostgreSQL and SearXNG using `docker compose -f infra/docker-compose.dev.yml up -d`.
3. Run the server: `cargo run -p malim-server`.
4. Run the web client: `npm install && npm run dev`.
5. For desktop development: `npm run tauri:dev`.

## Deployment

The production compose file is deliberately isolated from existing services. Configure a domain and secrets in `infra/.env`, review `infra/nginx/malim_chat.conf.example`, then deploy only after the host prerequisites are confirmed. It uses ports internal to the compose network; Nginx is the only public entry point.

For the current IP-only host deployment, the live application is served under the `/malim_chat/` path on the host. The native Android release must explicitly permit HTTP while this deployment remains IP-only; account credentials and provider keys should normally be sent over HTTPS once a domain is available.
