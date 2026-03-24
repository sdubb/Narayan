# Narayan UI

React frontend for the Narayan autonomous agent platform.

## Setup

```bash
npm install
cp .env.example .env
# Edit .env — set VITE_API_URL to your Narayan backend
npm run dev
```

## Pages

**Auth** — Register (creates tenant + shows API key once) or sign in with existing key to get a JWT.

**Chat** — Main interface:
- Sidebar lists all your agents with live status
- Select an agent to see its real-time SSE event stream
- Type a goal in the input bar to create a new agent
- Attach images for visual context (sent as base64 with the goal)
- Pause / resume agents from the header
- When an agent needs clarification, a form appears in the event feed
- When an agent calls `suggest_connectors`, connector cards appear inline
- Load a replay for completed agents via the event feed

**Settings** — Three tabs:
- **Credentials**: Add / remove provider keys (BYOK — your keys, encrypted at rest)
- **Routing**: Set which provider handles simple / medium / complex / fallback tasks
- **Usage**: Live metrics and per-provider token spend from the API

## Environment

```
VITE_API_URL=http://localhost:8080   # your Narayan backend
```

In development, Vite proxies `/api/*` to the backend so there are no CORS issues.
In production, set `VITE_API_URL` to your actual backend URL and deploy as a static site.

## Real API calls

All data is fetched from your Narayan instance:
- `GET /agents` — sidebar agent list
- `GET /agents/:id` — agent detail
- `GET /agents/:id/stream` — SSE event stream (live)
- `GET /agents/:id/replay` — past execution replay
- `POST /goals` — create new agent
- `POST /agents/:id/clarify` — submit clarification answers
- `POST /agents/:id/pause|resume` — agent control
- `GET /credentials` / `PUT /credentials` / `DELETE /credentials/:provider`
- `PUT /routing` — routing config
- `GET /metrics` + `GET /costs` — usage data

No mock data anywhere — if your backend is offline you'll see errors inline.
