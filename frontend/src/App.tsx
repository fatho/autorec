import React, { useEffect } from 'react';
import logo from './logo.svg';
import './App.css';
import Button from 'react-bootstrap/Button';

function App() {
  useEffect(() => {
    console.log(logo);
  });
  return (
    <div className="App">
      <header>
        <h1>AutoRec</h1>
        <Button>Hello World</Button>
      </header>
    </div>
  );
}

export default App;
