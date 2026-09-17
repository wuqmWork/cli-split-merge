/** @type {import('tailwindcss').Config} */
export default {
  darkMode: 'class',
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  theme: {
    extend: {
      boxShadow: {
        panel: '0 8px 24px rgba(15, 23, 42, 0.035)',
      },
    },
  },
  plugins: [],
};
