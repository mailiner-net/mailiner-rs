// tailwind.config.js
module.exports = {
  mode: "all",
  content: ["*.html"],
  theme: {
    extend: {
      colors: {
        // Primary palette (blue to turquoise)
        primary: {
          50: '#e6f9fa',
          100: '#ccf3f5',
          200: '#99e7eb',
          300: '#66dbe1',
          400: '#33cfd7',
          500: '#00c3cd', // Main primary color
          600: '#009ca4',
          700: '#00757b',
          800: '#004e52',
          900: '#002729',
        },
        // Secondary/neutral colors
        neutral: {
          50: '#f8fafc',
          100: '#f1f5f9',
          200: '#e2e8f0',
          300: '#cbd5e1',
          400: '#94a3b8',
          500: '#64748b',
          600: '#475569',
          700: '#334155',
          800: '#1e293b',
          900: '#0f172a',
        },
        // Danger colors (for delete actions)
        danger: {
          50: '#fef2f2',
          100: '#fee2e2',
          200: '#fecaca',
          300: '#fca5a5',
          400: '#f87171',
          500: '#ef4444', // Main danger color
          600: '#dc2626',
          700: '#b91c1c',
          800: '#991b1b',
          900: '#7f1d1d',
        },
        // Success colors (for confirmations)
        success: {
          50: '#f0fdf4',
          100: '#dcfce7',
          200: '#bbf7d0',
          300: '#86efac',
          400: '#4ade80',
          500: '#22c55e', // Main success color
          600: '#16a34a',
          700: '#15803d',
          800: '#166534',
          900: '#14532d',
        },
        // Warning colors
        warning: {
          400: '#facc15',
          500: '#eab308',
          600: '#ca8a04',
        },
      },
      borderRadius: {
        'sm': '0.2rem',      // Very subtle rounding
        DEFAULT: '0.3rem',   // Default subtle rounding
        'md': '0.4rem',      // Medium rounding
        'lg': '0.5rem',      // Larger rounding for dialogs
      },
      fontFamily: {
        sans: [
          'Inter',
          'system-ui',
          '-apple-system', 
          'BlinkMacSystemFont',
          '"Segoe UI"',
          'Roboto',
          '"Helvetica Neue"',
          'Arial',
          'sans-serif',
        ],
      },
      spacing: {
        // Standardized spacing scale
        'xs': '0.25rem',    // 4px
        'sm': '0.5rem',     // 8px
        'md': '0.75rem',    // 12px
        'base': '1rem',     // 16px
        'lg': '1.25rem',    // 20px
        'xl': '1.5rem',     // 24px
        '2xl': '2rem',      // 32px
      },
      boxShadow: {
        'sm': '0 1px 2px 0 rgba(0, 0, 0, 0.05)',
        DEFAULT: '0 1px 3px 0 rgba(0, 0, 0, 0.1), 0 1px 2px 0 rgba(0, 0, 0, 0.06)',
        'md': '0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06)',
        'lg': '0 10px 15px -3px rgba(0, 0, 0, 0.1), 0 4px 6px -2px rgba(0, 0, 0, 0.05)',
      },
    },
  },
  plugins: [
    require("daisyui"),
  ],
}
