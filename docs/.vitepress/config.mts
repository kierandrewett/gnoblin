import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Gnoblin',
  description: "Build a floating Wayland desktop with Gnoblin's compositor tools and public APIs.",
  base: '/gnoblin/',
  cleanUrls: true,
  themeConfig: {
    nav: [
      { text: 'Get started', link: '/' },
      { text: 'Config', link: '/config' },
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
          { text: 'Build your desktop', link: '/build-a-desktop' },
          { text: 'Troubleshooting', link: '/troubleshooting' },
        ],
      },
      {
        text: 'config',
        link: '/config',
        items: [
          { text: 'reference', link: '/config/reference' },
          { text: 'recipes', link: '/recipes' },
          {
            text: 'basics',
            collapsed: true,
            items: [
              { text: 'files_and_load_order', link: '/config/files_and_load_order' },
              { text: 'shortcuts', link: '/config/shortcuts' },
              { text: 'autostart', link: '/config/autostart' },
              { text: 'window_rules', link: '/config/window_rules' },
            ],
          },
          {
            text: 'appearance',
            collapsed: true,
            items: [
              { text: 'window_effects', link: '/config/window_effects' },
              { text: 'window_frames', link: '/config/window_frames' },
              { text: 'animations', link: '/config/animations' },
              { text: 'shaders', link: '/config/shaders' },
              { text: 'cursors', link: '/config/cursors' },
            ],
          },
          {
            text: 'behavior',
            collapsed: true,
            items: [
              { text: 'session_settings', link: '/config/session_settings' },
              { text: 'permissions', link: '/config/permissions' },
              { text: 'window_menu', link: '/config/window_menu' },
              { text: 'window_state_shortcuts', link: '/config/window_state_shortcuts' },
              { text: 'window_snapping', link: '/config/window_snapping' },
            ],
          },
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
