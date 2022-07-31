import React from "react";
import * as State from './State';

const AppContext = React.createContext({
    state: State.initialState,
    actions: State.actions,
    dispatch: (a: State.Action) => { },
});

type Props = {
    children?: React.ReactNode;
};

export const AppContextProvider = React.memo(({ children }: Props) => {
    const [state, dispatch] = React.useReducer(State.reducer, State.initialState);

    const eventSourceRef = React.useRef(null as EventSource | null);

    React.useEffect(() => {

        function connectEventSource() {
            if(eventSourceRef.current) {
                eventSourceRef.current.close();
            }
            const source = new EventSource("/updates-sse");

            eventSourceRef.current = source;

            source.onerror = e => {
                console.log(`EventSource failed ${e}`);
                setTimeout(connectEventSource, 1000);
            }
            source.onopen = e => {
                console.log(`EventSource opened`);
            }
            source.onmessage = e => {
                try {
                    const data = JSON.parse(e.data);
                    switch (data.type) {
                        case "PlayBegin":
                            dispatch({
                                type: State.ActionType.PlayStateUpdated,
                                recording_id: data.recording,
                            });
                            break;
                        case "PlayEnd":
                            dispatch({
                                type: State.ActionType.PlayStateUpdated,
                                recording_id: null,
                            });
                            break;
                        case "RecordBegin":
                            dispatch({
                                type: State.ActionType.RecordBegin,
                            });
                            break;
                        case "RecordEnd":
                            dispatch({
                                type: State.ActionType.RecordEnd,
                                recording: data.recording,
                            });
                            break;
                        case "RecordUpdate":
                            dispatch({
                                type: State.ActionType.RecordUpdate,
                                recording: data.recording,
                            });
                            break;
                        case "RecordDelete":
                            dispatch({
                                type: State.ActionType.RecordDelete,
                                recording_id: data.recording_id,
                            });
                            break;
                        case "RecordError":
                            dispatch({
                                type: State.ActionType.RecordError,
                                errorMessage: data.message,
                            });
                            break;
                    }
                    console.log(data);
                } catch (ex) {
                    console.log(`Malformed server message ${e.data}`);
                }
            };
        }

        connectEventSource();
        State.actions.queryRecordings(dispatch);
        State.actions.queryPlayState(dispatch);

        return () => {
            if(eventSourceRef.current) {
                console.log("Terminating EventSource");
                eventSourceRef.current.close();
            }
        };
    }, []);

    return (
        <AppContext.Provider value={{
            state, dispatch, actions: State.actions
        }}
        >
            {children}
        </AppContext.Provider>
    );
});

export const useAppContext = () => React.useContext(AppContext);
