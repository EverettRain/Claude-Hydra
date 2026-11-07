/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./src/index.html", "./src/**/*.{ts,tsx}"],
  darkMode: ["selector", ".dark"],
  theme: {
    extend: {
      colors: {
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        card: { DEFAULT: "hsl(var(--card))", foreground: "hsl(var(--card-foreground))" },
        primary: { DEFAULT: "hsl(var(--primary))", foreground: "hsl(var(--primary-foreground))" },
        muted: { DEFAULT: "hsl(var(--muted))", foreground: "hsl(var(--muted-foreground))" },
        accent: { DEFAULT: "hsl(var(--accent))", foreground: "hsl(var(--accent-foreground))" },
        destructive: { DEFAULT: "hsl(var(--destructive))", foreground: "hsl(var(--destructive-foreground))" },
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        blue: { 400: "#409CFF", 500: "#0A84FF", 600: "#0060DF", 700: "#0050BB" },
        gray: {
          50: "#fafafa", 100: "#f4f4f5", 200: "#e4e4e7", 300: "#d4d4d8", 400: "#a1a1aa",
          500: "#71717a", 600: "#636366", 700: "#48484A", 800: "#3A3A3C", 900: "#2C2C2E", 950: "#1C1C1E",
        },
      },
      borderRadius: { sm: "0.375rem", md: "0.5rem", lg: "0.75rem", xl: "0.875rem" },
      fontFamily: {
        sans: ["-apple-system", "BlinkMacSystemFont", '"Segoe UI"', '"Microsoft YaHei UI"', "Roboto", "Arial", "sans-serif"],
        mono: ["ui-monospace", "SFMono-Regular", "Consolas", "Menlo", "monospace"],
      },
      keyframes: {
        fadeIn: { "0%": { opacity: "0" }, "100%": { opacity: "1" } },
        popIn: { "0%": { opacity: "0", transform: "translateY(6px) scale(0.98)" }, "100%": { opacity: "1", transform: "none" } },
      },
      animation: { "fade-in": "fadeIn 0.15s ease-out", "pop-in": "popIn 0.16s ease-out" },
    },
  },
  plugins: [],
};
