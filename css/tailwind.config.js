/** @type {import('tailwindcss').Config} */
module.exports = {
  mode: "all",
  content: ["*.html"],
  theme: {
    extend: {},
    mailiner: {
      "primary": "#60a5fa",
      "secondary": "#e0f2fe",
      "accent": "#bae6fd",
      "neutral": "#d1d5db",
      "base-100": "#4b5563",
      "info": "#38bdf8",
      "success": "#22c55e",
      "warning": "#f59e0b",
      "error": "#e11d48",
    }
  },
  plugins: [
    require("daisyui")
  ],
};
