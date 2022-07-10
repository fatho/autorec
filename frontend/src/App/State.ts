import React from "react";

class AutoRecClient {
    constructor() {
    }

    async play_recording(recording: string) {
    }
}

enum ActionType {
    QueryRecordingsPending,
    QueryRecordingsFailed,
    QueryRecordingsSucceeded,

    PlayStateUpdated,
}

type Action = {
    type: ActionType,
    recordings?: Array<string>,
    errorMessage?: string,
    playing?: string | null,
}

enum PlayingState {
    Stopped,
    Playing,
    Pending,
}

type AppState = {
    recordings: Array<string>,
    recordingsLoading: boolean,
    error: boolean,
    errorMessage: string,
    playingState: PlayingState,
    playingRecording: string | null,
};

const initialState: AppState = {
    recordings: [],
    recordingsLoading: false,
    error: false,
    errorMessage: "",
    playingState: PlayingState.Pending,
    playingRecording: null,
};

type ActionDispatch = (a: Action) => void;

const actions = {
    queryRecordings: async (dispatch: ActionDispatch) => {
        dispatch({ type: ActionType.QueryRecordingsPending });
        try {
            const response = await fetch("/songs");
            const data = await response.json();
            if (response.status === 200) {
                dispatch({ type: ActionType.QueryRecordingsSucceeded, recordings: data });
            } else {
                dispatch({ type: ActionType.QueryRecordingsFailed, errorMessage: data });
            }
        } catch (e) {
            dispatch({ type: ActionType.QueryRecordingsFailed, errorMessage: (e as object).toString() });
        }
    },

    queryPlayState: async (dispatch: ActionDispatch) => {
        //dispatch({ type: ActionType.PlayStatePending });
        try {
            const response = await fetch("/play-status");
            const data = await response.json();
            dispatch({ type: ActionType.PlayStateUpdated, playing: data });
        } catch (e) {
            console.log(`Failed to get play state ${e}`)
            //dispatch({ type: ActionType.QueryRecordingsFailed, errorMessage: (e as object).toString() });
        }
    },

    playRecording: async (dispatch: ActionDispatch, recording: string) => {
        const response = await fetch("/play", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
          },
          body: JSON.stringify({ "name": recording })
        });
        // TODO: error handling

        // if (response.status < 200 || response.status >= 300) {
        //   const message = await response.text();
        //   setError(response.statusText + ': ' + message);
        // } else {
        //   setError(null);
        //   setPlaying(item);
        // }

    },

    stopPlaying: async (dispatch: ActionDispatch) => {
        const response = await fetch("/stop", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
          },
          body: JSON.stringify(null)
        });
        // TODO: error handling

    },
};

function reducer(state: AppState, action: Action): AppState {
    switch (action.type) {
        case ActionType.QueryRecordingsPending:
            return {
                ...state,
                recordingsLoading: true,
                error: false,
                errorMessage: "",
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
        case ActionType.PlayStateUpdated:
            return {
                ...state,
                playingRecording: action.playing!,
                playingState: action.playing! ? PlayingState.Playing : PlayingState.Stopped,
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