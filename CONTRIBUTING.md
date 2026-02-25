# Contributing

Thank you for your interest in contributing to tokscale!

## How to Contribute

### 1. Fork the repository

Click the **Fork** button on GitHub to create your own copy of the repo.

### 2. Clone and create a branch

```bash
git clone https://github.com/<your-username>/tokscale.git
cd tokscale
git checkout -b feat/your-feature-name
```

Use a descriptive branch name like `fix/parse-error` or `feat/add-export`.

### 3. Make your changes

Install dependencies and make sure everything works:

```bash
bun install
bun run build
```

Keep changes focused — one feature or fix per PR.

### 4. Commit your changes

Write clear, concise commit messages:

```bash
git commit -m "fix: handle missing token file gracefully"
```

### 5. Open a Pull Request

Push your branch and open a PR against `main`:

```bash
git push origin feat/your-feature-name
```

Then open a PR on GitHub. Describe what you changed and why.

## Guidelines

- Check existing [issues](https://github.com/junhoyeo/tokscale/issues) before opening a new one.
- For large changes, open an issue first to discuss the approach.
- Keep PRs small and reviewable.
