import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'

// No StrictMode: its dev double-mount tears down and re-creates the Aurora WebGL
// context, which can leave the surviving canvas with a lost context.
createRoot(document.getElementById('root')!).render(<App />)
