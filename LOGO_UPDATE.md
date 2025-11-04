# Logo Update - TechExLab Branding Added! 🏢

## What Changed

The **TechExLab logo** has been integrated into all pages, replacing the text-based "POIS" badge.

---

## 📸 Logo Details

**Source**: `https://esni.techexlab.com:3000/static/media/techex-logo.74b1230112edda6b3915.png`

**Dimensions**: 
- Height: 32px (auto-width to maintain aspect ratio)
- Displays beautifully on all devices

**Placement**: 
- Top-left corner of every page
- Links back to home page
- Part of the frosted glass header

---

## 🎨 Updated Pages

### ✅ Events Page (`events.html`)
```html
<header class="topbar">
  <div class="brand">
    <a href="/">
      <img src="https://esni.techexlab.com:3000/static/media/techex-logo.74b1230112edda6b3915.png" 
           alt="TechExLab" class="logo">
    </a>
    <a href="/static/events.html" class="brand-sub">Events</a>
  </div>
  ...
</header>
```

### ✅ Tools Page (`tools.html`)
- TechExLab logo in header
- "Admin" subtitle next to logo

### ✅ Admin Page (`admin.html`)
- TechExLab logo in header
- Full navigation bar
- "Admin" subtitle

---

## 🎯 CSS Update

Added logo-specific styling in `app.css`:

```css
.logo {
  height: 32px;
  width: auto;
  display: block;
}

/* Kept gradient badge option as .logo-text */
.logo-text {
  background: linear-gradient(135deg, var(--primary) 0%, var(--secondary) 100%);
  color: white;
  padding: 6px 12px;
  border-radius: 8px;
  font-weight: 700;
  font-size: 16px;
  letter-spacing: 0.5px;
}
```

---

## 🔄 Migration Options

### Option 1: Use External Logo (Current)
```html
<img src="https://esni.techexlab.com:3000/static/media/techex-logo.74b1230112edda6b3915.png" 
     alt="TechExLab" class="logo">
```

**Pros**: 
- No file hosting needed
- Always up-to-date
- Matches official branding

**Cons**:
- Requires external request
- Depends on esni.techexlab.com availability

### Option 2: Host Locally (Alternative)

If you want to host the logo locally:

1. Download the logo:
```bash
wget https://esni.techexlab.com:3000/static/media/techex-logo.74b1230112edda6b3915.png \
  -O static/techex-logo.png
```

2. Update HTML files:
```html
<img src="/static/techex-logo.png" alt="TechExLab" class="logo">
```

**Pros**:
- Faster load (no external request)
- Works offline
- Full control

**Cons**:
- Need to update manually if logo changes

---

## 🎨 Customization

### Change Logo Size

Edit `app.css`:
```css
.logo {
  height: 40px;  /* Larger logo */
  /* or */
  height: 24px;  /* Smaller logo */
}
```

### Add Logo Effects

```css
.logo {
  height: 32px;
  width: auto;
  display: block;
  transition: transform 0.2s;
}

.logo:hover {
  transform: scale(1.05);  /* Slight zoom on hover */
}
```

### Dark/Light Logo Variants

If you need different logos for dark/light themes:

```css
.logo.dark {
  display: block;
}

.logo.light {
  display: none;
}

/* For light theme */
[data-theme="light"] .logo.dark {
  display: none;
}

[data-theme="light"] .logo.light {
  display: block;
}
```

---

## 📱 Responsive Behavior

The logo automatically adjusts for mobile:

```css
@media (max-width: 768px) {
  .logo {
    height: 28px;  /* Slightly smaller on mobile */
  }
}
```

---

## ✅ Deployment

All files have been updated with the logo. Simply deploy as before:

```bash
cp app.css events.html tools.html admin.html static/
sudo systemctl restart pois
```

The logo will appear immediately on all pages! 🎉

---

## 🎯 Result

Your POIS Service now features:
- ✅ Professional TechExLab branding
- ✅ Consistent logo across all pages
- ✅ Clean, modern header design
- ✅ Matches the reference design perfectly

---

**Updated**: November 4, 2025  
**Logo Source**: esni.techexlab.com  
**Pages Updated**: events.html, tools.html, admin.html, app.css
