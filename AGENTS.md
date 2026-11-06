# ReactQM development rules

- Read `ROADMAP.md`, `README.md`, and `docs/ARCHITECTURE.md` before implementation. Keep actual progress and validation results current.
- Tauri 2 + Rust own Document / State / Renderer. WebView windows are projections of shared Rust state, not independent document owners.
- Use wgpu in Rust for heavy rendering / imaging / GPU work. Keep large arrays, volumes, trajectories and frame buffers in Rust / GPU / Worker storage. IPC carries commands and small state only.
- Keep quantum chemistry in the independent Python Worker. Do not replace validated engines with an ad hoc Schrödinger solver.
- Implement Adobe-style docking and floating panels incrementally, with Rust-owned layout and one shared document. The current shell does not implement these features yet.
- Add all UI strings in English, Japanese, and Simplified Chinese. Use icons and hover / keyboard-focus information affordances rather than lengthy explanations in primary UI.
- Follow Apple HIG where applicable. Check small window widths, meaningful line breaks, keyboard access, and light / dark appearance. Never claim visual verification from a successful build alone.
- Persist units, engine versions, calculation settings, convergence and provenance. Distinguish a method's numerical convergence from physical accuracy.
- Roadmap statuses: 🟢 [Done] implemented; 🟠 [Next] highest-priority unfinished; 🔴 [Later] subsequent planned; ⭕️ [Pending] cannot be validated in this environment.
- Keep local development independent of cloud infrastructure. If needed later, use XServer VPS Cloud (App VPS + Managed PostgreSQL + NFS) with separate VPS Workers communicating through HTTPS APIs / queues, without depending on direct Managed DB access.
- Validate changes with appropriate scripts in `package.json`; retain lockfiles for npm, Cargo and uv. GPU / native window validation may require access outside an execution sandbox.
- Do not report a dependency installation, stub, browser preview or synthetic fixture as a completed simulation feature.
