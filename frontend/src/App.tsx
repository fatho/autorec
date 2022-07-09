import React, { useEffect, useState } from 'react';
import './App.css';
import Button from 'react-bootstrap/Button';
import Alert from 'react-bootstrap/esm/Alert';
import Spinner from 'react-bootstrap/esm/Spinner';

function App() {
  return (
    <div className="App">
      <header>
        <h1>AutoRec</h1>
      </header>
      <SongList/>
    </div>
  );
}

function SongList() {
  const [songsLoading, setSongsLoading] = useState(true);
  const [songs, setSongs] = useState([]);
  const [error, setError] = useState((null as null | string));

  const fetchSongs = async() => {
    try {
      const response = await fetch("http://localhost:8000/songs");
      const data = await response.json();
      setSongs(data);
      setError(null);
    } catch(e) {
      if(e instanceof Error) {
        setError(e.message);
      } else {
        setError("Unknown error");
      }
    }
    setSongsLoading(false);
  };

  useEffect(() => {
    fetchSongs();
  }, []);

  if(songsLoading) {
    return (
      <Spinner animation="border" role="status">
        <span className="visually-hidden">Loading...</span>
      </Spinner>
    );
  } else {
    return (
      <div>
        {
          error
            ? (            
              <Alert key="error" variant="danger">
                Failed to fetch songs: {error}
              </Alert>
            )
            : songs.map(item => (
              <li>{item}</li>
            ))
        }
      </div>
    )
  }
}

export default App;
