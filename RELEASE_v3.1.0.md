# POIS v3.1.0 Release Notes

**Release Date:** November 14, 2024  
**Type:** Minor Release - UI/UX Improvements & Bug Fixes

---

## 🎯 Release Highlights

This release focuses on **UI consistency**, **authentication improvements**, and **fixing template rendering issues** across the POIS web interface. We've transitioned from server-side template rendering to a robust client-side header injection system that provides consistent branding and navigation across all pages.

---

## ✨ What's New

### 🎨 Unified Header System
- **Client-Side Header Injection**: All pages now use `header.js` for dynamic header rendering
- **Consistent Branding**: TxCue logo displayed consistently across all pages
- **No More Template Syntax Errors**: Fixed issue where pages displayed literal `{{include:_header.html}}` text
- **Responsive Navigation**: Header adapts to user role (admin vs regular user)

### 🔐 Enhanced JWT Authentication
- **Proper Authentication Flow**: Unauthenticated users redirected to login.html
- **Role-Based Navigation**: Admin-only pages (Users, API Tokens) hidden from non-admin users
- **Secure Token Validation**: JWT tokens validated on every page load
- **Username Display**: Current user displayed in header with logout functionality

### 📱 Updated Pages
All pages now use the unified header system:
- ✅ **admin.html** - Channels & Rules management
- ✅ **tools.html** - SCTE-35 Builder
- ✅ **events.html** - Event Monitor
- ✅ **users.html** - User Management (admin only)
- ✅ **tokens.html** - API Token Management (admin only)
- ✅ **docs.html** - API Documentation

### 🧭 Complete Navigation Menu
Six-item navigation consistently available to authenticated users:
1. **Channels & Rules** - Main admin interface
2. **SCTE-35 Builder** - Signal generation tool
3. **Event Monitor** - Real-time event tracking
4. **Users** - User management (admin only)
5. **API Tokens** - Token management (admin only)
6. **API Docs** - Complete API reference

---

## 🐛 Bug Fixes

### Critical Fixes
- **Fixed Template Rendering**: Resolved issue where static HTML files showed raw template syntax instead of rendered headers
- **Fixed Logo Display**: Corrected inconsistent logo display (TechExLab → TxCue)
- **Fixed 404 Errors**: Routes now properly redirect to working static files
- **Fixed Script Loading Order**: Ensured header.js loads before page-specific scripts

### Authentication Fixes
- **Fixed Auth Redirect Loop**: Users properly redirected to login when unauthenticated
- **Fixed Role Checking**: Admin-only pages correctly validate user roles
- **Fixed Token Validation**: JWT tokens properly validated on each request
- **Fixed Logout Flow**: Users properly redirected to login after logout

### UI/UX Fixes
- **Fixed Active State**: Current page properly highlighted in navigation
- **Fixed Header Layout**: Consistent header layout across all pages
- **Fixed Responsive Design**: Header properly adapts to different screen sizes
- **Fixed User Display**: Username properly displayed in header

---

## 🔧 Technical Changes

### Architecture Improvements
- **Shift from Server-Side to Client-Side Rendering**: Moved from Rust template engine to JavaScript header injection
- **Simplified Routing**: Static file serving instead of complex template handlers
- **Better Separation of Concerns**: Authentication logic centralized in header.js

### File Changes

#### New Files
- `static/header.js` (v3.0.2) - Client-side header injection with JWT auth

#### Updated Files
- `static/admin.html` (v3.0.2) - Uses header.js injection
- `static/tools.html` (v3.0.2) - Uses header.js injection
- `static/events.html` (v3.0.2) - Uses header.js injection
- `static/users.html` (v3.0.2) - Uses header.js injection
- `static/tokens.html` (v3.0.2) - Uses header.js injection
- `static/docs.html` (v3.0.3) - Uses header.js injection

#### Removed Dependencies
- Removed server-side template engine requirement
- Removed `templates.rs` module
- Simplified `main.rs` routing logic

---

## 📦 Deployment Notes

### Upgrade Path from v3.0.x

1. **Backup Current Files**
   ```bash
   sudo cp -r /opt/pois/static /opt/pois/static.backup
   ```

2. **Deploy Updated Files**
   ```bash
   # Copy all updated HTML files
   sudo cp admin.html tools.html events.html users.html tokens.html docs.html /opt/pois/static/
   sudo cp header.js /opt/pois/static/
   
   # Ensure correct ownership
   sudo chown -R pois:pois /opt/pois/static/
   ```

3. **Verify Logo File**
   ```bash
   ls -la /opt/pois/static/images/txcue-white.svg
   # Should exist with proper permissions
   ```

4. **Restart Service**
   ```bash
   sudo systemctl restart pois
   ```

5. **Clear Browser Cache**
   - Hard refresh (Ctrl+Shift+R / Cmd+Shift+R)
   - Or clear browser cache completely

### Verification Steps

1. **Test Authentication**
   - Navigate to `/` - should redirect to login if not authenticated
   - Login with credentials
   - Verify redirect to admin.html after successful login

2. **Test Navigation**
   - Verify all 6 navigation items appear (or 3 for non-admin)
   - Click each navigation item to verify routing
   - Verify active state highlights current page

3. **Test Role-Based Access**
   - Login as admin - should see Users and API Tokens
   - Login as regular user - should NOT see Users and API Tokens
   - Verify admin-only pages redirect non-admin users

4. **Test Logo Display**
   - Verify TxCue logo appears in header on all pages
   - Verify logo is properly sized and aligned
   - Verify logo is white/light colored for dark theme

---

## 🔄 Breaking Changes

### None
This release maintains backward compatibility with v3.0.x. All API endpoints remain unchanged.

---

## 📊 Performance Improvements

- **Faster Page Loads**: Client-side header injection eliminates server-side template processing
- **Reduced Server Load**: Static file serving more efficient than template rendering
- **Better Caching**: Static HTML files can be cached more effectively

---

## 🎓 Migration Examples

### Before (Server-Side Template)
```html
<body>
  {{include:_header.html}}
  <main>...</main>
</body>
```

### After (Client-Side Injection)
```html
<body>
  <!-- Header will be injected by header.js -->
  <main>...</main>
  <script src="/static/header.js"></script>
</body>
```

---

## 🔮 What's Next

### Planned for v3.2.0
- Enhanced event filtering and search
- Bulk rule operations
- Rule templates library
- Export/import functionality for channels and rules

### Under Consideration
- Dark/light theme toggle
- Customizable dashboard widgets
- Advanced SCTE-35 signal analysis
- Multi-language support

---

## 📚 Documentation

- **Installation Guide**: https://github.com/bokelleher/rust-pois/blob/main/README.md
- **API Documentation**: Available at `/static/docs.html` after deployment
- **Configuration Guide**: See `README.md` for configuration options

---

## 🙏 Acknowledgments

Thanks to all users who reported issues with template rendering and authentication flows. Your feedback was invaluable in identifying and resolving these critical UI/UX issues.

---

## 📝 Full Changelog

**UI/UX Improvements:**
- Unified header system with client-side injection
- Consistent TxCue branding across all pages
- Complete six-item navigation menu
- Role-based navigation display
- Active page highlighting

**Bug Fixes:**
- Fixed template syntax displaying in static files
- Fixed inconsistent logo display
- Fixed authentication redirect loops
- Fixed 404 errors on static routes
- Fixed script loading order issues
- Fixed active navigation state
- Fixed role-based page access

**Technical Changes:**
- Migrated from server-side to client-side header rendering
- Simplified routing architecture
- Centralized authentication logic
- Improved static file serving

**Files Changed:**
- Added: `static/header.js` (v3.0.2)
- Updated: `static/admin.html` (v3.0.2)
- Updated: `static/tools.html` (v3.0.2)
- Updated: `static/events.html` (v3.0.2)
- Updated: `static/users.html` (v3.0.2)
- Updated: `static/tokens.html` (v3.0.2)
- Updated: `static/docs.html` (v3.0.3)

---

**Version:** 3.1.0  
**Release Date:** November 14, 2024  
**GitHub:** https://github.com/bokelleher/rust-pois  
**License:** MIT

For questions or issues, please visit: https://github.com/bokelleher/rust-pois/issues
