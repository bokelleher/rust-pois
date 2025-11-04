# Contributing to POIS

Thank you for your interest in contributing to POIS (Placement Opportunity Information Service)!

## 🤝 Ways to Contribute

- **Bug Reports**: Found a bug? Open an issue with details
- **Feature Requests**: Have an idea? We'd love to hear it
- **Code Contributions**: Submit pull requests for fixes or features
- **Documentation**: Help improve or translate documentation
- **Testing**: Test on different platforms and report issues

## 🚀 Getting Started

### Prerequisites

- Rust 1.70+ (for backend)
- Basic knowledge of:
  - Rust/Axum for backend changes
  - HTML/CSS/JavaScript for UI changes
  - SCTE-35 and ESAM standards (for protocol work)

### Setup Development Environment

```bash
# Clone the repository
git clone https://github.com/bokelleher/rust-pois.git
cd rust-pois

# Build and run
cargo build
cargo run

# Or with environment variables
POIS_ADMIN_TOKEN=your-token cargo run
```

## 📝 Making Changes

### Code Style

**Rust Code:**
- Follow Rust standard formatting (`cargo fmt`)
- Run clippy before committing (`cargo clippy`)
- Add tests for new features

**Frontend Code:**
- Keep HTML semantic and accessible
- CSS: Use existing CSS variables for colors
- JavaScript: Keep it vanilla (no frameworks except Preact for admin.html)
- Comment complex logic

### Commit Messages

Use clear, descriptive commit messages:

```
feat: Add support for SCTE-35 bandwidth_reservation command
fix: Correct PTS time extraction in time_signal
docs: Update API documentation for /events endpoint
style: Format code with rustfmt
refactor: Simplify rule matching logic
test: Add tests for segmentation descriptor parsing
```

### Branch Naming

- `feature/description` - New features
- `fix/description` - Bug fixes
- `docs/description` - Documentation
- `refactor/description` - Code refactoring

## 🔍 Pull Request Process

1. **Fork the repository**
2. **Create a feature branch** from `main`
3. **Make your changes** with clear commits
4. **Test thoroughly**:
   - Run `cargo test`
   - Test manually with the UI
   - Check for edge cases
5. **Update documentation** if needed
6. **Submit pull request** with:
   - Clear description of changes
   - Link to related issues
   - Screenshots (for UI changes)

### Pull Request Checklist

- [ ] Code compiles without warnings
- [ ] Tests pass (`cargo test`)
- [ ] Formatting applied (`cargo fmt`)
- [ ] Clippy checks pass (`cargo clippy`)
- [ ] Documentation updated
- [ ] Commit messages are clear
- [ ] No sensitive data (tokens, passwords, etc.)

## 🐛 Reporting Bugs

When reporting bugs, include:

1. **Environment**:
   - OS and version
   - Rust version
   - POIS version/commit

2. **Steps to reproduce**:
   - Detailed steps
   - Sample ESAM requests if applicable

3. **Expected vs Actual behavior**

4. **Logs/Screenshots**:
   - Server logs
   - Browser console errors (for UI bugs)
   - Screenshots showing the issue

## 💡 Feature Requests

For feature requests:

1. **Check existing issues** first
2. **Describe the feature** clearly
3. **Explain the use case** - why is it needed?
4. **Provide examples** if applicable

## 🧪 Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture
```

### Adding Tests

Add tests for:
- New SCTE-35 command parsing
- Rule matching logic
- API endpoints
- Edge cases and error handling

Example:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_splice_insert() {
        // Your test here
    }
}
```

## 📚 Documentation

Help improve documentation by:

- Fixing typos or unclear instructions
- Adding examples
- Documenting edge cases
- Creating tutorials or guides
- Translating to other languages

## 🎨 UI/UX Contributions

For UI changes:

1. **Follow the design system** (see CUSTOMIZATION.md)
2. **Maintain dark theme** aesthetics
3. **Test responsive design** (mobile/tablet/desktop)
4. **Check accessibility** (contrast, keyboard navigation)
5. **Include screenshots** in PR

## 🔒 Security

If you discover a security vulnerability:

1. **Do NOT open a public issue**
2. **Email the maintainers** privately
3. **Provide detailed information**:
   - Vulnerability description
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

## 📜 Code of Conduct

### Our Standards

- Be respectful and inclusive
- Welcome newcomers
- Accept constructive criticism
- Focus on what's best for the project
- Show empathy towards others

### Unacceptable Behavior

- Harassment or discriminatory language
- Trolling or insulting comments
- Personal or political attacks
- Publishing others' private information
- Other unprofessional conduct

## ❓ Questions?

- **General questions**: Open a GitHub Discussion
- **Bug reports**: Open an Issue
- **Feature requests**: Open an Issue
- **Security concerns**: Email maintainers

## 📄 License

By contributing, you agree that your contributions will be licensed under the MIT License.

## 🙏 Recognition

Contributors are recognized in:
- GitHub contributors page
- Release notes
- README.md (for significant contributions)

Thank you for contributing to POIS! 🚀
