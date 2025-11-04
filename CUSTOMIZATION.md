# 🎨 Customization Guide

This guide shows you how to customize the POIS interface with your own branding and colors.

---

## 🏢 Branding Options

### Option 1: Add Your Logo Image (Recommended)

1. **Add your logo file** to the `static/` directory:
   ```bash
   cp your-logo.png static/logo.png
   ```

2. **Update HTML files** (events.html, tools.html, admin.html):
   ```html
   <!-- Uncomment this line: -->
   <img src="/static/logo.png" alt="Your Company" class="logo">
   
   <!-- Comment out the text logo: -->
   <!-- <span class="logo-text">POIS</span> -->
   ```

3. **Adjust logo size** in `app.css` if needed:
   ```css
   .logo {
     height: 40px;  /* Adjust as needed */
     width: auto;
   }
   ```

### Option 2: Use Text Logo (Default)

The default setup uses a gradient text logo:

```html
<span class="logo-text">POIS</span>
```

**Customize the text:**
```html
<span class="logo-text">YourBrand</span>
```

---

## 🎨 Color Scheme Customization

Edit `static/app.css` to change the color scheme:

### Purple-Blue Theme (Default)
```css
:root {
  --primary:   #667eea;  /* Purple-blue */
  --secondary: #764ba2;  /* Deep purple */
  --success:   #48bb78;  /* Green */
  --warning:   #ffd93d;  /* Yellow */
  --error:     #ff6b6b;  /* Red */
}
```

### Example: Green Tech Theme
```css
:root {
  --primary:   #10b981;  /* Emerald green */
  --secondary: #059669;  /* Dark green */
  --success:   #22c55e;  /* Success green */
  --warning:   #f59e0b;  /* Amber */
  --error:     #ef4444;  /* Red */
}
```

### Example: Blue Corporate Theme
```css
:root {
  --primary:   #3b82f6;  /* Blue */
  --secondary: #1e40af;  /* Dark blue */
  --success:   #10b981;  /* Green */
  --warning:   #f59e0b;  /* Amber */
  --error:     #ef4444;  /* Red */
}
```

### Example: Orange Energy Theme
```css
:root {
  --primary:   #f97316;  /* Orange */
  --secondary: #ea580c;  /* Dark orange */
  --success:   #22c55e;  /* Green */
  --warning:   #eab308;  /* Yellow */
  --error:     #dc2626;  /* Red */
}
```

---

## 🌈 Background Gradient

Change the dark gradient background in `app.css`:

```css
body {
  /* Default: Dark blue gradient */
  background: linear-gradient(135deg, #0a0a0a 0%, #1a1a2e 100%);
  
  /* Alternative: Pure dark */
  /* background: linear-gradient(135deg, #0a0a0a 0%, #1a1a1a 100%); */
  
  /* Alternative: Dark purple */
  /* background: linear-gradient(135deg, #1a0a1a 0%, #2a1a3a 100%); */
  
  /* Alternative: Dark green */
  /* background: linear-gradient(135deg, #0a1a0a 0%, #1a2a1a 100%); */
}
```

---

## 🔤 Font Customization

Change the font family in `app.css`:

```css
body {
  font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  
  /* Alternative: Google Fonts (add <link> to HTML head) */
  /* font-family: 'Inter', sans-serif; */
  
  /* Alternative: Monospace for technical look */
  /* font-family: 'SF Mono', Monaco, 'Cascadia Code', monospace; */
}
```

To use Google Fonts, add to HTML `<head>`:
```html
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
```

---

## 📝 Page Titles & Subtitles

### Change Page Titles

In each HTML file's `<title>` tag:

**events.html:**
```html
<title>Your Company - Event Monitor</title>
```

**tools.html:**
```html
<title>Your Company - SCTE-35 Builder</title>
```

**admin.html:**
```html
<title>Your Company - Admin</title>
```

### Change Subtitle Text

In the header brand section:

```html
<a href="/static/events.html" class="brand-sub">Your Subtitle</a>
```

Examples:
- "Dashboard"
- "Control Panel"
- "Monitoring"
- "Events"

---

## 🎯 Panel Styling

### Adjust Frosted Glass Opacity

In `app.css`:

```css
:root {
  /* More opaque (less transparent) */
  --panel: rgba(20, 20, 35, 0.98);
  
  /* More transparent */
  --panel: rgba(20, 20, 35, 0.85);
  
  /* Completely opaque */
  --panel: rgba(20, 20, 35, 1.0);
}
```

### Adjust Backdrop Blur

```css
.panel, .card {
  backdrop-filter: blur(8px);   /* Default */
  /* backdrop-filter: blur(4px);  /* Less blur */
  /* backdrop-filter: blur(12px); /* More blur */
  /* backdrop-filter: none;       /* No blur */
}
```

---

## 🔘 Button Styles

### Change Primary Button Gradient

In `app.css`:

```css
.btn-primary {
  /* Default: Purple gradient */
  background: linear-gradient(135deg, var(--primary), var(--secondary));
  
  /* Alternative: Solid color */
  /* background: var(--primary); */
  
  /* Alternative: Reverse gradient */
  /* background: linear-gradient(135deg, var(--secondary), var(--primary)); */
}
```

---

## 📐 Spacing & Sizing

### Adjust Border Radius (Roundness)

```css
:root {
  --radius: 12px;  /* Default - very rounded */
  /* --radius: 6px;  /* Less rounded */
  /* --radius: 16px; /* More rounded */
  /* --radius: 4px;  /* Subtle rounding */
}
```

### Adjust Header Height

```css
:root {
  --topbar-h: 64px;  /* Default */
  /* --topbar-h: 56px;  /* Compact */
  /* --topbar-h: 72px;  /* Spacious */
}
```

---

## 🖼️ Complete Rebrand Example

Here's a complete example for rebranding to "Acme Corp":

### 1. Add Logo
```bash
cp acme-logo.png static/logo.png
```

### 2. Update HTML Files
```html
<img src="/static/logo.png" alt="Acme Corp" class="logo">
<a href="/static/events.html" class="brand-sub">Monitoring</a>
```

### 3. Update Colors (app.css)
```css
:root {
  --primary:   #0066cc;  /* Acme blue */
  --secondary: #003d7a;  /* Dark blue */
  --success:   #00a86b;  /* Acme green */
  --warning:   #ff9500;  /* Orange */
  --error:     #d62828;  /* Red */
}
```

### 4. Update Page Titles
```html
<title>Acme Corp - Event Monitor</title>
```

---

## 🎨 CSS Variable Reference

All customizable variables in `app.css`:

```css
:root {
  /* Colors */
  --primary:   #667eea;
  --secondary: #764ba2;
  --success:   #48bb78;
  --warning:   #ffd93d;
  --error:     #ff6b6b;

  /* Text */
  --text-primary:  #e0e0e0;
  --text-secondary:#a0a0a0;

  /* Backgrounds */
  --dark:    #0a0a0a;
  --panel:   rgba(20, 20, 35, 0.95);
  --border:  rgba(255,255,255,0.10);

  /* Form Fields */
  --field-bg: rgba(255,255,255,0.05);
  --field-bg-hover: rgba(255,255,255,0.08);
  
  /* Layout */
  --topbar-h: 64px;
  --radius: 12px;
  --shadow: 0 1px 2px rgba(0,0,0,.05);
}
```

---

## 📦 Quick Theme Switcher

To offer multiple themes, add this to your HTML:

```html
<script>
  // Theme presets
  const themes = {
    purple: {
      primary: '#667eea',
      secondary: '#764ba2'
    },
    blue: {
      primary: '#3b82f6',
      secondary: '#1e40af'
    },
    green: {
      primary: '#10b981',
      secondary: '#059669'
    }
  };

  function setTheme(themeName) {
    const theme = themes[themeName];
    document.documentElement.style.setProperty('--primary', theme.primary);
    document.documentElement.style.setProperty('--secondary', theme.secondary);
    localStorage.setItem('theme', themeName);
  }

  // Load saved theme
  const saved = localStorage.getItem('theme');
  if (saved) setTheme(saved);
</script>
```

Then add theme buttons:
```html
<button onclick="setTheme('purple')">Purple</button>
<button onclick="setTheme('blue')">Blue</button>
<button onclick="setTheme('green')">Green</button>
```

---

## 🎯 Best Practices

1. **Logo Format**: Use PNG with transparent background (recommended size: 32px height)
2. **Color Contrast**: Ensure text is readable on backgrounds (use contrast checkers)
3. **Consistent Branding**: Use the same colors across all pages
4. **Test Mobile**: Check your branding looks good on small screens
5. **Performance**: Optimize logo file size for faster loading

---

## 📸 Preview Your Changes

After making changes:

1. Clear browser cache (Ctrl+Shift+R)
2. Check all pages:
   - Admin panel (`/static/admin.html`)
   - Event Monitor (`/static/events.html`)
   - SCTE-35 Builder (`/static/tools.html`)
3. Test on mobile devices
4. Verify logo displays correctly
5. Check color contrast in DevTools

---

## 🆘 Troubleshooting

**Logo not showing?**
- Check file path: `/static/logo.png`
- Verify file permissions (readable)
- Check browser console for 404 errors

**Colors not changing?**
- Clear browser cache
- Check CSS syntax (no typos)
- Verify `:root` selector in app.css

**Gradient not working?**
- Some older browsers may not support backdrop-filter
- Provide fallback colors in those cases

---

**Need help?** Open an issue on GitHub with screenshots of your desired customization!
