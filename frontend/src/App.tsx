import React, { KeyboardEvent, useEffect, useRef, useState } from 'react';
import logo from './logo.svg';
import './App.css';

// Icons
import { ArrowClockwise, StopFill, PlayFill, VolumeUp, Trash, Pencil, ClockHistory, MusicNote } from 'react-bootstrap-icons';

import { AppContextProvider, useAppContext } from './App/AppContext';
import { PlayingState, Recording, RecordingId } from './App/State';
import { Button, Alert, Spinner, ButtonToolbar, ButtonGroup, Navbar, Container, ListGroup, Stack, Modal, Nav, Offcanvas, ListGroupItem } from 'react-bootstrap';

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
        <Container className="mt-2 px-0 px-sm-2">
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
  const [itemToDelete, setItemToDelete] = useState(null as number | null);

  const [showConfirmRename, setShowConfirmRename] = useState(false);
  const [itemToRename, setItemToRename] = useState(null as number | null);
  const [newName, setNewName] = useState("");
  const [nameSuggestions, setNameSuggestions] = useState(null as {name: string, similarity: number}[] | null);
  const renameInput = useRef(null as HTMLInputElement | null);

  const handleCloseDelete = () => setShowConfirmDelete(false);
  const handleCloseRename = () => setShowConfirmRename(false);

  function handleDelete() {
    if(itemToDelete) {
      actions.deleteRecording(dispatch, itemToDelete);
    }
    setShowConfirmDelete(false);
  }

  function handleRename(overrideNewName?: string) {
    const initial = state.recordings.find(rec => rec.id === itemToRename);
    if(itemToRename && initial) {
      actions.updateRecording(dispatch, {
        ...initial,
        name: overrideNewName || newName
      });
    }
    setShowConfirmRename(false);
  }

  function handleRenameKeyUp(e: KeyboardEvent<HTMLInputElement>) {
    if(e.key === "Enter") {
      handleRename();
    }
  }

  function confirmDelete(item: RecordingId) {
    setItemToDelete(item);
    setShowConfirmDelete(true);
  }

  function confirmRename(item: RecordingId) {
    const initial = state.recordings.find(rec => rec.id === item);
    if(initial) {
      setNewName(initial.name);
      setItemToRename(item);
      setNameSuggestions(null);
      fetch(`/recordings/${item}/classify`, {
        method: 'POST'
      }).then(res => res.json()).then((data: {name: string, similarity: number}[]) =>
        setNameSuggestions(data.filter(suggestion => suggestion.similarity > 0.9))
      );
      setShowConfirmRename(true);      
    }
  }
  
  useEffect(
    () => {
      if(showConfirmRename && renameInput.current) {
        renameInput.current.focus();
      }
    },
    [showConfirmRename, renameInput]
  )

  const groups: { title: string, items: Recording[] }[] = [];

  var currentGroup = null as string | null;
  var currentGroupItems: Recording[] = [];
  state.recordings.forEach((item) => {
    const group = item.created_at.toLocaleDateString(undefined, {
      weekday: 'short', year: 'numeric', month: 'long', day: 'numeric'
    });
    if(currentGroup === group) {
      currentGroupItems.push(item);
    } else {
      currentGroup = group;
      currentGroupItems = [item];
      groups.push({
        title: currentGroup,
        items: currentGroupItems,
      });
    }
  });
  
  return (
    <>
      <Modal show={showConfirmDelete} onHide={handleCloseDelete}>
        <Modal.Header closeButton>
          <Modal.Title>Confirm deletion</Modal.Title>
        </Modal.Header>
        <Modal.Body>Delete recording {itemToDelete}?</Modal.Body>
        <Modal.Footer>
          <Button variant="secondary" onClick={handleCloseDelete}>
            Cancel
          </Button>
          <Button variant="danger" onClick={handleDelete}>
            Delete
          </Button>
        </Modal.Footer>
      </Modal>
      
      <Modal show={showConfirmRename} onHide={handleCloseRename}>
        <Modal.Header closeButton>
          <Modal.Title>Rename recording</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <input
            ref={renameInput}
            placeholder="Enter name here or select below"
            type="text"
            className="form-control"
            onKeyUp={handleRenameKeyUp}
            value={newName}
            onChange={e => setNewName(e.target.value)}
            />
            {
              nameSuggestions !== null
                ? (
                  <>
                    <hr/>
                    {
                      nameSuggestions.length > 0 
                        ? (

                            <ListGroup>
                              {
                                nameSuggestions.slice(0, 3).map(suggestion => (
                                  <ListGroupItem action key={suggestion.name} variant="primary" onClick={e => {
                                      handleRename(suggestion.name);
                                    }}>
                                    {suggestion.name}
                                  </ListGroupItem>
                                ))
                              }
                            </ListGroup>
                          )
                        : <>No similar songs</>
                    }
                  </>
                )
                : <Spinner animation='border' />
            }
        </Modal.Body>
        <Modal.Footer>
          <Button variant="secondary" onClick={handleCloseRename}>
            Cancel
          </Button>
          <Button variant="primary" onClick={e => handleRename()}>
            Rename
          </Button>
        </Modal.Footer>
      </Modal>

      <Stack>
        {state.isRecording
          ? (
            <ListGroup>
              <ListGroup.Item key="recording">
                <Stack direction='horizontal'>
                  <Spinner animation='grow' variant="danger" />
                  <div className="ms-2">Recording in progress</div>
                </Stack>
              </ListGroup.Item>
            </ListGroup>
          )
          : <></>}
        {groups.map(group => (
          <ListGroup key={group.title} className="mt-2">
            <ListGroup.Item key="title">
              <b>{group.title}</b>
            </ListGroup.Item>
            {group.items.map(item => (
            <ListGroup.Item key={item.id}>
              <RecordingItem
                recording={item}
                playingState={state.playingState === PlayingState.Pending && state.playingQueued === item.id
                  ? PlayingState.Pending
                  : (state.playingRecording === item.id ? PlayingState.Playing : PlayingState.Stopped)}
                onPlay={() => actions.playRecording(dispatch, item.id)}
                onStop={() => actions.stopPlaying(dispatch)}
                onRequestDelete={() => confirmDelete(item.id)}
                onRequestRename={() => confirmRename(item.id)}
                />
            </ListGroup.Item>
          ))}
          </ListGroup>
        ))}
      </Stack></>
  )
}

type RecordingItemProps = {
  recording: Recording,
  playingState: PlayingState,
  onPlay: () => void,
  onStop: () => void,
  onRequestDelete: () => void,
  onRequestRename: () => void,
};

const RecordingItem = React.memo((props: RecordingItemProps) => {
  function playControlButton() {
    switch (props.playingState) {
      case PlayingState.Pending:
        return (<Button variant="outline-primary" disabled><Spinner size="sm" animation="border" /></Button>)
      case PlayingState.Playing:
        return (<Button variant="primary" onClick={props.onStop}><StopFill /></Button>)
      case PlayingState.Stopped:
        return (<Button variant="outline-primary" onClick={props.onPlay}><PlayFill /></Button>)
    }
  }

  function prettySeconds(total_seconds: number): string {
    let minutes = Math.floor(total_seconds / 60);
    let seconds = Math.round(total_seconds - minutes * 60);
    
    return `${minutes.toFixed(0)}:` + (seconds < 10 ? "0" : "") + `${seconds.toFixed(0)}`;
  }

  return (
    <Stack direction='horizontal'>
      <div aria-label="Click to Rename" onClick={props.onRequestRename} className="text-truncate me-auto">
          {
            props.recording.name
              ? (<span>{props.recording.name}</span>)
              : (<span className="text-italic">Unnamed</span>)
          }
          {
            props.playingState === PlayingState.Playing
              ? <VolumeUp className="ms-2" size="1.5em" color="gray" />
              : <></>
          }
          <footer className="text-muted text-smaller">
            <span className="me-2">{props.recording.created_at.toLocaleTimeString()}</span>
            <span className="me-1"><ClockHistory/> {prettySeconds(props.recording.length_seconds)}</span>
            <span className="me-1"><MusicNote/> {props.recording.note_count}</span>
          </footer>
      </div>
      <Button onClick={props.onRequestDelete} variant="outline-danger" className="ms-2 me-2"><Trash /></Button>
      {playControlButton()}
    </Stack>
  );
  //<Button onClick={props.onRequestDelete} variant="outline-primary" className="me-2"><Pencil /></Button>
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
              <Button variant="outline-primary" onClick={() => {
                actions.queryRecordings(dispatch);
                actions.queryPlayState(dispatch);
              }}><ArrowClockwise /></Button>
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
