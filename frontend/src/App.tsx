import React, { useEffect, useState } from 'react';
import logo from './logo.svg';
import './App.css';

// Icons
import { ArrowClockwise, StopFill, PlayFill, Steam, Boombox, VolumeUp } from 'react-bootstrap-icons';

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

import { AppContextProvider, useAppContext } from './App/AppContext';
import { PlayingState } from './App/State';

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
      <AppContextProvider>
        <Container>
          <Toolbar />
          <ErrorBanner />
          <RecordingsList />
        </Container>
      </AppContextProvider>
    </div>
  );
}

function RecordingsList() {
  const { state, actions, dispatch } = useAppContext();

  return (
    <ListGroup>
      {
        state.recordings.map(item => (
          <ListGroup.Item key={item}>
            <RecordingItem
              recording={item}
              playingState={
                state.playingState === PlayingState.Pending && state.playingQueued === item
                  ? PlayingState.Pending
                  : (state.playingRecording === item ? PlayingState.Playing : PlayingState.Stopped)
              }
              onPlay={ () => actions.playRecording(dispatch, item) }
              onStop={ () => actions.stopPlaying(dispatch) }
              />
          </ListGroup.Item>
        ))
      }
    </ListGroup>
  )
}

type RecordingItemProps = {
  recording: string,
  playingState: PlayingState,
  onPlay: () => void,
  onStop: () => void,
};

const RecordingItem = React.memo((props: RecordingItemProps) => {
  function button() {
    switch(props.playingState) {
      case PlayingState.Pending:
        return (<Button disabled><Spinner size="sm" animation="border" /></Button>)
      case PlayingState.Playing:
        return (<Button onClick={props.onStop}><StopFill /></Button>)
      case PlayingState.Stopped:
        return (<Button onClick={props.onPlay}><PlayFill /></Button>)
    }
  }
  return (
    <Stack direction='horizontal'>
      <div className="text-truncate">{props.recording}</div>
      {
        props.playingState == PlayingState.Playing
          ? <VolumeUp size="1.5em" />
          : <></>
      }
      <div className="ms-auto"></div>
      { button() }
    </Stack>
  );
});


function ErrorBanner() {
  const { state } = useAppContext();

  return state.error ? (
    <Alert key="error" variant="danger">
      {state.errorMessage}
    </Alert>
  ) : <></>
}

function Toolbar() {
  const { state, actions, dispatch } = useAppContext();

  return (
    <ButtonToolbar className="pt-2 pb-2" aria-label="Song control">
      <ButtonGroup className="me-2" aria-label="First group">
        {
          state.recordingsLoading
            ? (
              <Button variant="secondary" disabled>
                <Spinner animation="border" role="status" size="sm" />
              </Button>
            )
            : (
              <Button variant="secondary" onClick={() => actions.queryRecordings(dispatch)}><ArrowClockwise /></Button>
            )
        }
      </ButtonGroup>
      <ButtonGroup className="me-2" aria-label="Second group">
        {
          state.playingState === PlayingState.Pending
            ? (<Button variant="secondary" disabled><Spinner animation="border" size="sm" /></Button>)
            : (<Button variant="secondary" disabled={state.playingState === PlayingState.Stopped} onClick={() => actions.stopPlaying(dispatch)}><StopFill /></Button>)
        }
      </ButtonGroup>
    </ButtonToolbar>
  )
}

export default App;
