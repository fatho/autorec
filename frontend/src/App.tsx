import React, { useEffect, useState } from 'react';
import logo from './logo.svg';
import './App.css';

// Icons
import { ArrowClockwise, StopFill, PlayFill } from 'react-bootstrap-icons';

import Button from 'react-bootstrap/Button';
import Alert from 'react-bootstrap/esm/Alert';
import Spinner from 'react-bootstrap/esm/Spinner';
import ButtonToolbar from 'react-bootstrap/esm/ButtonToolbar';
import ButtonGroup from 'react-bootstrap/esm/ButtonGroup';
import Navbar from 'react-bootstrap/esm/Navbar';
import Container from 'react-bootstrap/esm/Container';
import Row from 'react-bootstrap/esm/Row';
import ListGroup from 'react-bootstrap/esm/ListGroup';
import Stack from 'react-bootstrap/esm/Stack';


function App() {
  return (
    <div className="App">
      <Navbar bg="light" variant="light">
        <Container>
          <Navbar.Brand href="#home">
            <img
              alt=""
              src={logo}
              width="30"
              height="30"
              className="d-inline-block align-top"
            />{' '}
            AutoRec
          </Navbar.Brand>
        </Container>
      </Navbar>
      <SongList />
    </div>
  );
}

function SongList() {
  const [songsLoading, setSongsLoading] = useState(false);
  const [controlLoading, setControlLoading] = useState(null as null | string);
  const [songs, setSongs] = useState([]);
  const [error, setError] = useState(null as null | string);
  const [playing, setPlaying] = useState(null as null | string);

  const fetchSongs = async () => {
    try {
      setSongsLoading(true);
      const response = await fetch("/songs");
      const data = await response.json();
      setSongs(data);
      setError(null);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Unknown error");
      }
    }
    setSongsLoading(false);
  };

  const playSong = async (item: string) => {
    setControlLoading(item);
    try {
      const response = await fetch("/play", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ "name": item })
      });
      if (response.status < 200 || response.status >= 300) {
        const message = await response.text();
        setError(response.statusText + ': ' + message);
      } else {
        setError(null);
        setPlaying(item);
      }
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Unknown error");
      }
    }
    setControlLoading(null);
  }

  const stopSong = async () => {
    setControlLoading(playing);
    try {
      setPlaying(null);

      const response = await fetch("/stop", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify(null)
      });
      if (response.status < 200 || response.status >= 300) {
        const message = await response.text();
        setError(response.statusText + ': ' + message);
      } else {
        setError(null);
      }
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Unknown error");
      }
    }
    setControlLoading(null);
  }

  const updatePlaying = async () => {
    try {
      const response = await fetch("/play-status");
      const data = await response.json();
      setPlaying(data);
    } catch (e) {
      if (e instanceof Error) {
        console.log("Failed to poll playing status" + e.message);
      } else {
        console.log("Failed to poll playing status" + e);
      }
    }
  };

  useEffect(() => {
    fetchSongs();
  }, []);

  useEffect(() => {
    updatePlaying();
    const timer = setInterval(updatePlaying, 2000);
    return () => {
      clearInterval(timer);
    }
  }, []);

  return (
    <Container className="border">
      <Row>
        <ButtonToolbar className="mt-2 mb-3" aria-label="Song control">
          <ButtonGroup className="me-2" aria-label="First group">
            <Button variant="secondary" onClick={fetchSongs}><ArrowClockwise /></Button>
          </ButtonGroup>
          <ButtonGroup className="me-2" aria-label="Second group">
            {
              controlLoading !== null
                ? (<Button variant="secondary" disabled><Spinner animation="border" size="sm" /></Button>)
                : (<Button variant="secondary" disabled={playing === null} onClick={stopSong}><StopFill /></Button>)
            }
          </ButtonGroup>
          {
            songsLoading
              ? (
                <ButtonGroup className="me-2">
                  <Spinner animation="border" role="status">
                    <span className="visually-hidden">Loading...</span>
                  </Spinner>
                </ButtonGroup>
              )
              : null
          }
        </ButtonToolbar>
      </Row>
      {
        error
          ? (
            <Row>
              <Alert key="error" variant="danger">
                {error}
              </Alert>
            </Row>
          )
          : null
      }
      <Row className="ms-0">
        <ListGroup>
          {
            songs.map(item => (
              <ListGroup.Item key={item}>
                <Stack direction='horizontal'>
                  <div className="text-truncate">{item}</div>
                  <div className="ms-auto"></div>
                  {
                    controlLoading === item
                      ? (<Button disabled><Spinner animation="border" size="sm" /></Button>)
                      :
                        (playing === item
                          ? (<Button disabled={controlLoading !== null} onClick={stopSong}><StopFill /></Button>)
                          : (<Button disabled={controlLoading !== null} onClick={() => playSong(item)}><PlayFill /></Button>))
                  }
                </Stack>
              </ListGroup.Item>
            ))
          }
        </ListGroup>
      </Row>
    </Container>
  )
}

export default App;
