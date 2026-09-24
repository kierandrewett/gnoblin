import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Gnoblin',
  description: "Build a floating Wayland desktop with Gnoblin's compositor tools and public APIs.",
  base: '/gnoblin/',
  cleanUrls: true,
  themeConfig: {
    nav: [
      { text: 'Get started', link: '/' },
      { text: 'Configuration', link: '/configuration-reference' },
      { text: 'Shell development', link: '/shell-integration' },
      { text: 'Contributing', link: '/source-development' },
      { text: 'GitHub', link: 'https://github.com/kierandrewett/gnoblin' },
    ],
    search: { provider: 'local' },
    outline: { level: [2, 3] },
    editLink: {
      pattern: 'https://github.com/kierandrewett/gnoblin/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/kierandrewett/gnoblin' },
    ],
    sidebar: [
      {
        text: 'Get started',
        items: [
          { text: 'Welcome', link: '/' },
          {
            text: 'Installation',
            collapsed: true,
            items: [
              { text: 'Choose your system', link: '/installation' },
              { text: 'Fedora', link: '/install-fedora' },
              { text: 'Debian / Ubuntu', link: '/install-debian' },
              { text: 'NixOS', link: '/install-nixos' },
              { text: 'Build from source', link: '/install-source' },
            ],
          },
          { text: 'Choose a shell', link: '/bring-your-own-shell' },
          { text: 'Configure Gnoblin', link: '/configuration' },
          { text: 'Build your desktop', link: '/build-a-desktop' },
          { text: 'Troubleshooting', link: '/troubleshooting' },
        ],
      },
      {
        text: 'Configuration',
        items: [
          { text: 'All settings', link: '/configuration-reference' },
          { text: 'Recipes', link: '/configuration-recipes' },
          {
            text: 'Basics',
            collapsed: true,
            items: [
              { text: 'Files and load order', link: '/configuration-loading' },
              { text: 'Shortcuts', link: '/shortcuts' },
              { text: 'Autostart', link: '/autostart' },
              { text: 'Window rules', link: '/window-rules' },
            ],
          },
          {
            text: 'Appearance',
            collapsed: true,
            items: [
              { text: 'Effects', link: '/window-effects' },
              { text: 'Titlebars', link: '/window-frames' },
              { text: 'Animations', link: '/animations' },
              { text: 'Shaders', link: '/shaders' },
              { text: 'Cursors', link: '/cursors' },
            ],
          },
          {
            text: 'Behavior',
            collapsed: true,
            items: [
              { text: 'Session settings', link: '/session-settings' },
              { text: 'Permissions', link: '/permissions' },
              { text: 'Window menu', link: '/window-menu' },
              { text: 'Restore or minimise', link: '/window-state-shortcuts' },
              { text: 'Snapping', link: '/window-snapping' },
            ],
          },
          { text: 'Command line', link: '/gnoblinctl' },
        ],
      },
      {
        text: 'Shell development',
        collapsed: true,
        items: [
          { text: 'Integration', link: '/shell-integration' },
          { text: 'Compositor bridge', link: '/compositor-bridge' },
          { text: 'Bridge examples', link: '/bridge-examples' },
          { text: 'Wayland protocols', link: '/wayland-protocols' },
          { text: 'Session locking design', link: '/session-lock-design' },
          { text: 'User scripts', link: '/user-scripts' },
          { text: 'Background blur', link: '/background-effects' },
          { text: 'Blur fades', link: '/blur-fades' },
          { text: 'Effect rendering', link: '/effects-rendering' },
          { text: 'Activation', link: '/focus-transfer' },
          { text: 'Launch feedback', link: '/launch-feedback' },
          { text: 'Frame architecture', link: '/window-frame-renderers' },
          { text: 'Write a renderer', link: '/frame-renderer-api' },
          { text: 'Native and external UI', link: '/native-ui-removal' },
          { text: 'Console', link: '/developer-console' },
        ],
      },
      {
        text: 'Contributing',
        collapsed: true,
        items: [
          { text: 'Source development', link: '/source-development' },
          { text: 'Devkit', link: '/devkit' },
          { text: 'Testing', link: '/testing' },
          { text: 'Desktop verification', link: '/real-hardware-verification' },
          { text: 'Formatting', link: '/code-quality' },
          { text: 'CLI development', link: '/cli-development' },
          { text: 'Packaging', link: '/distribution' },
          { text: 'Documentation', link: '/documentation-site' },
          { text: 'Archive', link: '/archive' },
        ],
      },
    ],
    docFooter: { prev: 'Previous page', next: 'Next page' },
  },
  markdown: {
    theme: {
      light: 'github-light',
      dark: 'github-dark',
    },
  },
})
