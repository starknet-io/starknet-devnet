/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        starknet: {
          blue: '#080A18',
          purple: '#B76CFF',
          accent: '#FF7A61',
          mint: '#35D6B4',
          gold: '#F4C95D',
          surface: '#101521',
          card: '#151B2A',
          border: '#2A3446',
        },
      },
      boxShadow: {
        panel: '0 20px 60px rgba(0, 0, 0, 0.28)',
        glow: '0 0 0 1px rgba(183, 108, 255, 0.16), 0 18px 50px rgba(183, 108, 255, 0.08)',
      },
    },
  },
  plugins: [],
};
