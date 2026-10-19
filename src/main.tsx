import React from 'react';
import {createRoot} from 'react-dom/client';
import {App} from './app/App';
import {Startup} from './components/Startup';
import '@fontsource/jetbrains-mono/latin-400.css';
import '@fontsource/jetbrains-mono/latin-500.css';
import '@fontsource/jetbrains-mono/latin-600.css';

const container=document.getElementById('root');
if(container)createRoot(container).render(<Startup><App/></Startup>);
