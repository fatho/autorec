enum ActionType {
    QueryRecordingsPending,
    QueryRecordingsFailed,
    QueryRecordingsSucceeded,

    RecordBegin,
    RecordEnd,

    PlayControlPending,
    PlayStateUpdated,
    PlayStateFailed,
}

type Recording = string;

type Action = {
    type: ActionType,
    recordings?: Array<Recording>,
    recording?: Recording,
    errorMessage?: string,
    playing?: string | null,
}

enum PlayingState {
    Stopped,
    Playing,
    Pending,
}

type AppState = {
    recordings: Array<Recording>,
    recordingsLoading: boolean,

    error: boolean,
    errorMessage: string,

    playingState: PlayingState,
    playingRecording: string | null,
    playingQueued: string | null,

    isRecording: boolean,
};

const initialState: AppState = {
    recordings: [],
    recordingsLoading: false,

    error: false,
    errorMessage: "",

    playingState: PlayingState.Pending,
    playingRecording: null,
    playingQueued: null,

    isRecording: false,
};

type ActionDispatch = (a: Action) => void;

class StatusError extends Error {
}

async function checkForStatus(response: Response) {
    if(response.status < 200 || response.status >= 300) {
        const data = await response.text();
        try {
            const json = JSON.parse(data);
            if(typeof json === "string") {
                throw new StatusError(json);
            } else {
                throw new StatusError(json.message);
            }
        } catch(e) {
            throw new StatusError(data);
        }
    }
}

const actions = {
    queryRecordings: async (dispatch: ActionDispatch) => {
        dispatch({ type: ActionType.QueryRecordingsPending });
        try {
            const response = await fetch("/songs");
            await checkForStatus(response);
            const data = await response.json();
            dispatch({ type: ActionType.QueryRecordingsSucceeded, recordings: data });
        } catch (e) {
            dispatch({ type: ActionType.QueryRecordingsFailed, errorMessage: (e as object).toString() });
        }
    },

    queryPlayState: async (dispatch: ActionDispatch) => {
        //dispatch({ type: ActionType.PlayStatePending });
        try {
            const response = await fetch("/play-status");
            await checkForStatus(response);
            const data = await response.json();
            dispatch({ type: ActionType.PlayStateUpdated, playing: data });
        } catch (e) {
            dispatch({ type: ActionType.PlayStateFailed, errorMessage: (e as object).toString() });
        }
    },

    playRecording: async (dispatch: ActionDispatch, recording: string) => {
        dispatch({ type: ActionType.PlayControlPending, playing: recording });
        try {
            const response = await fetch("/play", {
              method: "POST",
              headers: {
                "Content-Type": "application/json",
              },
              body: JSON.stringify({ "name": recording })
            });
            await checkForStatus(response);
        } catch (e) {
            dispatch({ type: ActionType.PlayStateFailed, errorMessage: (e as object).toString() });
        }
    },

    stopPlaying: async (dispatch: ActionDispatch) => {
        dispatch({ type: ActionType.PlayControlPending, playing: null });
        try {
            const response = await fetch("/stop", {
              method: "POST",
              headers: {
                "Content-Type": "application/json",
              },
              body: JSON.stringify(null)
            });
            await checkForStatus(response);
        } catch (e) {
            dispatch({ type: ActionType.PlayStateFailed, errorMessage: (e as object).toString() });
        }
    },
};

function reducer(state: AppState, action: Action): AppState {
    switch (action.type) {
        case ActionType.QueryRecordingsPending:
            return {
                ...state,
                recordingsLoading: true,
            }
        case ActionType.QueryRecordingsSucceeded:
            return {
                ...state,
                recordings: action.recordings!,
                recordingsLoading: false,
                error: false,
                errorMessage: "",
            }
        case ActionType.QueryRecordingsFailed:
            return {
                ...state,
                recordingsLoading: false,
                error: true,
                errorMessage: action.errorMessage!,
            }

        case ActionType.RecordBegin:
            return {
                ...state,
                isRecording: true,
            }

        case ActionType.RecordEnd:
            return {
                ...state,
                recordings: [action.recording!, ...state.recordings],
                isRecording: false,
            }
        
        case ActionType.PlayStateUpdated:
            return {
                ...state,
                playingRecording: action.playing!,
                playingState: action.playing! ? PlayingState.Playing : PlayingState.Stopped,
            }
        case ActionType.PlayControlPending:
            return {
                ...state,
                playingState: PlayingState.Pending,
                playingQueued: action.playing!,
            }
        case ActionType.PlayStateFailed:
            return {
                ...state,
                playingState: PlayingState.Stopped,
                playingQueued: null,
                playingRecording: null,
                error: true,
                errorMessage: action.errorMessage!,
            }
        default:
            throw new Error(`Unknown action type: ${action.type}`)
    }
}



export {
    initialState,
    reducer,
    actions,

    type ActionDispatch,
    type Action,
    ActionType,

    PlayingState,
    type AppState,
};