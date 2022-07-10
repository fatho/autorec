import React, { useEffect, useState } from 'react';
import logo from './logo.svg';
import './App.css';

// Icons
import { ArrowClockwise, StopFill, PlayFill, VolumeUp, Trash } from 'react-bootstrap-icons';

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
import { Form, FormControl, Modal, Nav, NavDropdown, Offcanvas } from 'react-bootstrap';

function App() {
  return (
    <div className="App">
      <AppContextProvider>
        <Navbar expand={false} bg="light" variant="light" sticky="top" className="border">
          <Container>
            <Navbar.Toggle aria-controls="nav-offcanvasNavbar" />
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
            <Navbar.Offcanvas
              id="nav-offcanvasNavbar"
              aria-labelledby="nav-offcanvasNavbarLabel"
              placement="start"
            >
              <Offcanvas.Header closeButton>
                <Offcanvas.Title id="nav-offcanvasNavbarLabel">
                  <img
                    alt=""
                    src={logo}
                    width="30"
                    height="30"
                    className="d-inline-block align-top"
                  />{' '}
                  Menu
                </Offcanvas.Title>
              </Offcanvas.Header>
              <Offcanvas.Body>
                <Nav>
                  <Nav.Item>
                    <Nav.Link href="#bydate">Recordings by Date</Nav.Link>
                  </Nav.Item>
                  <Nav.Item>
                    <Nav.Link href="#about">About</Nav.Link>
                  </Nav.Item>
                </Nav>
              </Offcanvas.Body>
            </Navbar.Offcanvas>
            <Toolbar className="ms-2 ms-sm-0 my-2" />
          </Container>
        </Navbar>
        <Container className="px-0 px-sm-2">
          <ErrorBanner />
          <RecordingsList />
        </Container>
      </AppContextProvider>
    </div>
  );
}

function RecordingsList() {
  const { state, actions, dispatch } = useAppContext();

  const [showConfirmDelete, setShowConfirmDelete] = useState(false);
  const [itemToDelete, setItemToDelete] = useState("");

  const handleClose = () => setShowConfirmDelete(false);

  function confirmDelete(item: string) {
    setItemToDelete(item);
    setShowConfirmDelete(true);
  }

  return (
    <>
      <Modal show={showConfirmDelete} onHide={handleClose}>
        <Modal.Header closeButton>
          <Modal.Title>Confirm deletion</Modal.Title>
        </Modal.Header>
        <Modal.Body>Delete recording {itemToDelete}?</Modal.Body>
        <Modal.Footer>
          <Button variant="secondary" onClick={handleClose}>
            Cancel
          </Button>
          <Button variant="danger" onClick={handleClose}>
            Delete
          </Button>
        </Modal.Footer>
      </Modal>

      <ListGroup>
        {state.isRecording
          ? (
            <ListGroup.Item key="recording">
              <Stack direction='horizontal'>
                <Spinner animation='grow' variant="danger" />
                <div className="ms-2">Recording in progress</div>
              </Stack>
            </ListGroup.Item>
          )
          : <></>}
        {state.recordings.map(item => (
          <ListGroup.Item key={item}>
            <RecordingItem
              recording={item}
              playingState={state.playingState === PlayingState.Pending && state.playingQueued === item
                ? PlayingState.Pending
                : (state.playingRecording === item ? PlayingState.Playing : PlayingState.Stopped)}
              onPlay={() => actions.playRecording(dispatch, item)}
              onStop={() => actions.stopPlaying(dispatch)}
              onRequestDelete={() => confirmDelete(item)} />
          </ListGroup.Item>
        ))}
      </ListGroup></>
  )
}

type RecordingItemProps = {
  recording: string,
  playingState: PlayingState,
  onPlay: () => void,
  onStop: () => void,
  onRequestDelete: () => void,
};

const RecordingItem = React.memo((props: RecordingItemProps) => {
  function button() {
    switch (props.playingState) {
      case PlayingState.Pending:
        return (<Button variant="outline-primary" disabled><Spinner size="sm" animation="border" /></Button>)
      case PlayingState.Playing:
        return (<Button variant="primary" onClick={props.onStop}><StopFill /></Button>)
      case PlayingState.Stopped:
        return (<Button variant="outline-primary" onClick={props.onPlay}><PlayFill /></Button>)
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
      <Button onClick={props.onRequestDelete} variant="outline-danger" className="me-2"><Trash /></Button>
      {button()}
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



function Toolbar({ className }: { className: string }) {
  const { state, actions, dispatch } = useAppContext();

  return (
    <ButtonToolbar className={className} aria-label="Song control">
      <ButtonGroup className="me-2" aria-label="First group">
        {
          state.recordingsLoading
            ? (
              <Button variant="outline-primary" disabled>
                <Spinner animation="border" role="status" size="sm" />
              </Button>
            )
            : (
              <Button variant="outline-primary" onClick={() => actions.queryRecordings(dispatch)}><ArrowClockwise /></Button>
            )
        }
      </ButtonGroup>
      <ButtonGroup className="me-1" aria-label="Second group">
        {
          state.playingState === PlayingState.Pending
            ? (<Button variant="outline-primary" disabled><Spinner animation="border" size="sm" /></Button>)
            : (<Button
                variant={state.playingState === PlayingState.Stopped ? "secondary" : "outline-primary" }
                disabled={state.playingState === PlayingState.Stopped}
                onClick={() => actions.stopPlaying(dispatch)}><StopFill /></Button>)
        }
      </ButtonGroup>
    </ButtonToolbar>
  )
}

export default App;
