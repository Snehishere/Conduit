# Contributing to Conduit

Thank you for considering contributing to Conduit! This document provides guidelines and instructions for contributing.

---

## Code of Conduct

Be respectful, inclusive, and constructive. We're building something useful together.

---

## How to Contribute

### Reporting Bugs

1. Check if the bug has already been reported in [Issues](https://github.com/Snehishere/Conduit/issues)
2. If not, create a new issue with:
   - Clear title describing the problem
   - Steps to reproduce
   - Expected behavior vs actual behavior
   - Screenshots (if applicable)
   - Platform/version information

### Suggesting Features

1. Check [Discussions](https://github.com/Snehishere/Conduit/discussions) to see if it's been suggested
2. Open a new discussion with:
   - Clear description of the feature
   - Use case and benefits
   - Any implementation ideas (optional)

### Submitting Code

1. **Fork the repository**
2. **Create a feature branch**: `git checkout -b feature/your-feature-name`
3. **Make your changes**
4. **Test thoroughly** on your platform
5. **Commit with clear messages**: `git commit -m "Add feature: description"`
6. **Push to your fork**: `git push origin feature/your-feature-name`
7. **Open a Pull Request** with:
   - Description of changes
   - Related issue number (if applicable)
   - Screenshots/demos (if UI changes)

---

## Development Setup

### Desktop

```bash
cd desktop
npm install
npm run dev
```

### Mobile

```bash
cd mobile
flutter pub get
flutter run
```

---

## Coding Standards

### Desktop (TypeScript/React)
- Use TypeScript strict mode
- Follow existing code style
- Add comments for complex logic
- Use meaningful variable names

### Desktop (Rust)
- Run `cargo fmt` before committing
- Run `cargo clippy` and fix warnings
- Write unit tests for new functionality
- Avoid `unwrap()` — use proper error handling

### Mobile (Flutter/Dart)
- Follow [Effective Dart](https://dart.dev/guides/language/effective-dart) guidelines
- Run `flutter analyze` before committing
- Format code with `dart format`
- Write widget tests for UI components

---

## Testing

- Write tests for new features
- Ensure existing tests pass
- Test on multiple platforms if possible
- Manual testing checklist:
  - [ ] Device pairing works
  - [ ] Clipboard sync works
  - [ ] File transfers complete
  - [ ] No console errors
  - [ ] UI looks correct

---

## Pull Request Guidelines

- Keep PRs focused on a single feature/fix
- Update documentation if needed
- Add screenshots for UI changes
- Reference related issues
- Be responsive to feedback

---

## First-Time Contributors

Look for issues labeled `good first issue` — these are beginner-friendly tasks that are a great way to get started!

---

## Questions?

Feel free to ask in [GitHub Discussions](https://github.com/Snehishere/Conduit/discussions) or open an issue.

---

Thank you for contributing! 🎉
