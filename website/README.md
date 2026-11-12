# Limusic website

Static landing page (Vite + React + Tailwind + React Bits), deployed to GitHub Pages at https://limusic.app (custom domain, DNS on Cloudflare) by .github/workflows/website.yml on every push to master touching website/.

```bash
pnpm install
pnpm dev      # local dev
pnpm build    # outputs dist/
```

`/lyrics/` is a second page: the Limusic Lyrics leaderboard, and a contributor's profile at `/lyrics/?u=<handle>`. It reads the lyrics server's public endpoints. To run it against a local server, start that server with `WEB_ORIGINS=http://localhost:5173` and run `VITE_LYRICS_URL=http://127.0.0.1:<port> pnpm dev`.
