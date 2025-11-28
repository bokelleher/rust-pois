// header.js - Common header with JWT authentication
// Version: 3.1.0
// Last updated: 2024-11-19
// Changes: Added settings gear icon, removed tokens from nav, admin.html → channels.html
(function() {
  'use strict';

  // Helper functions
  function getToken() {
    return localStorage.getItem('pois_token');
  }

  function setToken(token) {
    if (token) {
      localStorage.setItem('pois_token', token);
    } else {
      localStorage.removeItem('pois_token');
      localStorage.removeItem('pois_user');
    }
  }

  function getUser() {
    const userStr = localStorage.getItem('pois_user');
    if (!userStr) return null;
    try {
      return JSON.parse(userStr);
    } catch (e) {
      return null;
    }
  }

  function isAuthenticated() {
    const token = getToken();
    if (!token) return false;

    try {
      // Decode JWT to check expiration
      const payload = JSON.parse(atob(token.split('.')[1]));
      const now = Math.floor(Date.now() / 1000);
      return payload.exp > now;
    } catch (e) {
      return false;
    }
  }

  function requireAuth() {
    if (!isAuthenticated()) {
      // Redirect to login if not authenticated
      if (window.location.pathname !== '/static/login.html') {
        window.location.href = '/static/login.html';
      }
      return false;
    }
    return true;
  }

  function logout() {
    setToken(null);
    window.location.href = '/static/login.html';
  }

  // Determine active page
  function getActivePage() {
    const path = window.location.pathname;
    if (path.includes('channels.html') || path.includes('admin.html')) return 'channels';
    if (path.includes('tools.html')) return 'tools';
    if (path.includes('events.html')) return 'events';
    if (path.includes('users.html')) return 'users';
    if (path.includes('settings.html')) return 'settings';
    if (path.includes('docs.html')) return 'docs';
    return '';
  }

  function renderHeader() {
    // Skip on login page
    if (window.location.pathname.includes('login.html')) {
      return;
    }

    // Require authentication for all pages except login
    if (!requireAuth()) {
      return;
    }

    const user = getUser();
    const isAdmin = user && user.role === 'admin';
    const username = user ? user.username : 'User';
    const activePage = getActivePage();

    const headerHTML = `
      <header class="topbar">
        <div class="brand">
          <a href="/">
            <img src="/static/images/txcue-white.svg" alt="TxCue" class="logo">
          </a>
        </div>
        <nav class="nav">
          <a href="/static/channels.html" ${activePage === 'channels' ? 'class="active"' : ''}>Channels &amp; Rules</a>
          <a href="/static/tools.html" ${activePage === 'tools' ? 'class="active"' : ''}>SCTE-35 Tools</a>
          <a href="/static/events.html" ${activePage === 'events' ? 'class="active"' : ''}>Event Monitor</a>
          ${isAdmin ? `<a href="/static/users.html" ${activePage === 'users' ? 'class="active"' : ''}>Users</a>` : ''}
          <a href="/static/docs.html" ${activePage === 'docs' ? 'class="active"' : ''}>Docs</a>
        </nav>
        <div class="spacer"></div>
        <div class="right">
          <span style="color: var(--text-secondary); font-size: 16px; display: flex; align-items: center; gap: 8px;">
            ${username}
            <a href="/static/settings.html" 
               style="color: var(--text-secondary); text-decoration: none; display: flex; align-items: center;" 
               title="Settings">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="3"></circle>
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path>
              </svg>
            </a>
          </span>
          <button class="btn" id="logoutBtn">Logout</button>
        </div>
      </header>
    `;

    // Find where to inject the header
    const existingHeader = document.querySelector('header.topbar');
    if (existingHeader) {
      existingHeader.outerHTML = headerHTML;
    } else {
      // Insert at top of body
      document.body.insertAdjacentHTML('afterbegin', headerHTML);
    }

    // Attach logout handler
    const logoutBtn = document.getElementById('logoutBtn');
    if (logoutBtn) {
      logoutBtn.addEventListener('click', logout);
    }
  }

  // Run when DOM is ready
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', renderHeader);
  } else {
    renderHeader();
  }

  // Export global API for other scripts
  window.POIS_AUTH = {
    getToken,
    setToken,
    getUser,
    isAuthenticated,
    requireAuth,
    logout
  };

})();