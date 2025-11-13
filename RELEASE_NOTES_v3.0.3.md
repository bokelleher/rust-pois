# Version 3.0.3 - Authentication & Multi-Tenancy

**Release Date**: November 13, 2025

## 🔐 Major Release: JWT Authentication System

This release introduces a complete authentication and authorization system with JWT tokens and multi-tenancy support. POIS now supports multiple users with isolated data and token-based API access.

---

## ✨ New Features

### Authentication System
- **JWT-based authentication** - Secure token-based auth for all API endpoints
- **User management** - Create and manage user accounts with passwords
- **Login system** - Web-based login interface with session management
- **Token management** - Generate and revoke API tokens per user
- **Multi-tenancy** - Complete data isolation between users

### New UI Pages
- **Login page** (`/login.html`) - User authentication interface
- **Users page** (`/users.html`) - User account management (admin)
- **Tokens page** (`/tokens.html`) - API token generation and management
- **Test token page** (`/test_token.html`) - Token validation testing

### Security Features
- **Password hashing** - Bcrypt-based secure password storage
- **Token expiration** - Configurable JWT token lifetimes
- **Bearer token auth** - Standard HTTP Authorization header support
- **Session management** - Browser localStorage-based sessions

---

## 🔧 Technical Improvements

### Backend Architecture
- Added `jwt_auth.rs` - JWT token generation and validation
- Added `auth_handlers.rs` - Authentication endpoint handlers
- Added `models.rs` enhancements for User and ApiToken models
- Updated all API endpoints with authentication middleware
- Added user context propagation throughout request lifecycle

### Database Schema
- New `users` table with username, email, password hash
- New `api_tokens` table with user association and expiry
- Migration `0003_add_jwt_auth.sql` - JWT auth schema
- Migration `0004_add_multitenancy.sql` - User isolation for channels/rules/events
- Foreign key constraints for data isolation

### API Changes
- **Breaking Change**: All `/api/*` endpoints now require authentication
- New endpoint: `POST /api/auth/login` - User login
- New endpoint: `POST /api/auth/register` - User registration
- New endpoint: `GET /api/auth/validate` - Token validation
- New endpoint: `POST /api/tokens` - Generate API token
- New endpoint: `GET /api/tokens` - List user's tokens
- New endpoint: `DELETE /api/tokens/:id` - Revoke token
- New endpoint: `GET /api/users` - List users (admin)
- New endpoint: `POST /api/users` - Create user (admin)

### Configuration
- Updated `install.sh` with JWT secret generation
- Added `POIS_JWT_SECRET` environment variable support
- Enhanced systemd service configuration

---

## 🐛 Bug Fixes

- Fixed event logging to include user context
- Fixed channel/rule ownership and access control
- Improved error handling for authentication failures

---

## 📦 Files Changed

### New Files
- `src/jwt_auth.rs` - JWT token management
- `src/auth_handlers.rs` - Authentication endpoints
- `migrations/0003_add_jwt_auth.sql` - Auth schema
- `migrations/0004_add_multitenancy.sql` - Multi-tenancy schema
- `static/login.html` - Login interface
- `static/users.html` - User management
- `static/tokens.html` - Token management
- `static/test_token.html` - Token testing
- `static/images/` - UI assets
- `static/site.webmanifest` - PWA manifest

### Updated Files
- `src/main.rs` - Version 3.0.3, auth middleware integration
- `src/models.rs` - User and ApiToken models
- `static/admin.html` - Auth-aware admin interface
- `static/events.html` - Auth-aware monitoring
- `static/tools.html` - Auth-aware tools
- `static/docs.html` - Updated documentation
- `install.sh` - JWT secret generation

### Removed Files
- `static/openapi.yaml` - Replaced with integrated docs
- `LOGO_UPDATE.md` - Temporary documentation
- `USE_CASES.md` - Consolidated into main docs

---

## 🔄 Migration from v2.1.0

### ⚠️ Breaking Changes

**Authentication Required**: All API endpoints now require authentication. Existing API scripts must be updated.

### Migration Steps

1. **Backup your database**:
   ```bash
   cp /opt/pois/pois.db /opt/pois/pois.db.backup
   ```

2. **Update the installation**:
   ```bash
   cd rust-pois
   git pull
   sudo systemctl stop pois
   sudo ./install.sh
   ```
   
   **Note**: If this is a fresh database, the installer will prompt you to create an admin user:
   - Admin username (default: `admin`)
   - Admin email (default: `admin@example.com`)
   - Admin password (minimum 8 characters, with confirmation)

3. **Log in to the web UI**:
   - Navigate to `https://your-server/login.html`
   - Enter your admin credentials
   - You're now authenticated!

4. **Generate API token** (for scripts):
   - After logging in, go to "Tokens" page
   - Click "Generate Token"
   - Copy the token for use in API scripts
   - Update scripts with: `Authorization: Bearer YOUR_TOKEN`

5. **Update API scripts**:
   ```bash
   # Old (v2.1.0)
   curl -H "Authorization: Bearer dev-token" http://localhost:8080/api/channels
   
   # New (v3.0.3)
   curl -H "Authorization: Bearer YOUR_JWT_TOKEN" https://localhost:8080/api/channels
   ```

### Database Migrations

Migrations run automatically on startup:
- `0003_add_jwt_auth.sql` - Creates users and api_tokens tables
- `0004_add_multitenancy.sql` - Adds user_id to channels, rules, events

**Note**: Existing data (channels/rules/events created in v2.x) will be associated with the first user created.

---

## 🔒 Security Notes

- **Change JWT Secret**: Set a strong `POIS_JWT_SECRET` in production
- **Use HTTPS**: JWT tokens should only be transmitted over TLS
- **Token Storage**: Web UI stores tokens in localStorage (consider HttpOnly cookies for enhanced security)
- **Password Policy**: Implement strong password requirements in production
- **Token Rotation**: Regularly rotate API tokens

---

## 🚀 What's Next

Future releases will focus on:
- Rule template library (v3.1.0)
- Enhanced user roles and permissions
- Token refresh mechanism
- Audit logging
- OAuth2 integration

---

## 📚 Documentation

- [Installation Guide](INSTALL.md)
- [API Documentation](https://pois.techexlab.com/docs.html)
- [Contributing Guidelines](CONTRIBUTING.md)
- [Customization Guide](CUSTOMIZATION.md)

---

## 🙏 Acknowledgments

Thank you to all users who provided feedback on authentication requirements and multi-tenancy support!

---

**Full Changelog**: https://github.com/bokelleher/rust-pois/compare/v2.1.0...v3.0.3
