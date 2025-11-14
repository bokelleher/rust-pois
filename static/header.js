// header.js - Common header with JWT authentication
// Version: 3.0.1
// Last updated: 2024-11-14
// Changes: Fixed template literal syntax error on line 87
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
    if (path.includes('admin.html')) return 'admin';
    if (path.includes('tools.html')) return 'tools';
    if (path.includes('events.html')) return 'events';
    if (path.includes('users.html')) return 'users';
    if (path.includes('tokens.html')) return 'tokens';
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
            <img src="/static/images/txcue-white.svg" alt="txcue" class="logo">
          </a>
        </div>
        <nav class="nav">
          <a href="/static/admin.html" ${activePage === 'admin' ? 'class="active"' : ''}>Channels &amp; Rules</a>
          <a href="/static/tools.html" ${activePage === 'tools' ? 'class="active"' : ''}>SCTE-35 Builder</a>
          <a href="/static/events.html" ${activePage === 'events' ? 'class="active"' : ''}>Event Monitor</a>
          ${isAdmin ? `<a href="/static/users.html" ${activePage === 'users' ? 'class="active"' : ''}>Users</a>` : ''}
          <a href="/static/tokens.html" ${activePage === 'tokens' ? 'class="active"' : ''}>API Tokens</a>
          <a href="/static/docs.html" ${activePage === 'docs' ? 'class="active"' : ''}>API Docs</a>
        </nav>
        <div class="spacer"></div>
        <div class="right">
          <span id="tokenDisplay" style="color: var(--text-secondary); font-size: 13px;">
            ${username}
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