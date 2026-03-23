# Odyssey Docs

The docs site is a Docusaurus app under `docs/`.

Content lives in:

- `docs/content/index.mdx` for the landing page
- `docs/content/guides/` for user-facing walkthroughs
- `docs/content/reference/` for command, manifest, sandbox, and API references
- `docs/content/runtime/` for execution model and internal architecture
- `docs/sidebars.ts` for sidebar ordering

Local development:

```bash
cd docs
npm install
npm start
```

Production build with the GitHub Pages base path:

```bash
cd docs
npm run build
```

Before shipping doc changes:

- build the site with `npm run build`
- check that sidebar links and slugs still match
- update generated-user docs such as `crates/odyssey-rs-runtime/configs/README.md` when template behavior changes

Released Rust API docs live on docs.rs.
