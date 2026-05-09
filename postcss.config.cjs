// Must use .cjs extension — project uses "type":"module" in package.json,
// so PostCSS requires a CommonJS config (not treated as ESM).
module.exports = {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
};
