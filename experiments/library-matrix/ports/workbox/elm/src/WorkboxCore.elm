module WorkboxCore exposing (workboxError, copyResponse, cacheMatchIgnoreParams, waitUntil, timeout)

{-| workbox-core: WorkboxError, copyResponse, cacheMatchIgnoreParams, `waitUntil(event, asyncFn)`
and `timeout`.
-}

import Host exposing (Cache, Request, Response)
import Io exposing (Io, Promise, Thrown(..))
import Json.Decode as D
import Json.Encode as E
import World exposing (Event, Wb)


{-| `new WorkboxError(code, details)`, with the details in insertion order.
-}
workboxError : String -> List ( String, E.Value ) -> Thrown
workboxError code details =
    WorkboxError code (Just (E.object details))


{-| `waitUntil(event, asyncFn)`: the started call extends the event's lifetime and is returned.
-}
waitUntil : Event -> (a -> E.Value) -> D.Decoder a -> Wb a -> Wb (Promise a)
waitUntil event encode decoder asyncFn =
    Io.spawn encode decoder asyncFn
        |> Io.andThen (\returnPromise -> World.eventWaitUntil event (Io.promiseId returnPromise) |> Io.map (\() -> returnPromise))


{-| `timeout(0)`: one timer task.
-}
timeout : Io w ()
timeout =
    Io.sleep


{-| `copyResponse(response)`: refuses a response from another origin (or without a URL), then
builds a new Response from a clone's headers, status, status text and body. Workbox's
`canConstructResponseFromBodyStream` always answers false, so the body is read as a Blob.
-}
copyResponse : Response -> Wb Response
copyResponse response =
    (if String.isEmpty response.url then
        Io.succeed Nothing

     else
        Host.newUrl response.url Nothing |> Io.map (.origin >> Just)
    )
        |> Io.andThen
            (\origin ->
                Io.get (.location >> .origin)
                    |> Io.andThen
                        (\locationOrigin ->
                            if origin /= Just locationOrigin then
                                Io.throw (workboxError "cross-origin-copy-response" [ ( "origin", origin |> Maybe.map E.string |> Maybe.withDefault E.null ) ])

                            else
                                Host.cloneResponse response
                                    |> Io.andThen
                                        (\clonedResponse ->
                                            Host.responseHeaders clonedResponse
                                                |> Io.andThen
                                                    (\headers ->
                                                        Host.responseBlob clonedResponse
                                                            |> Io.andThen (\body -> Host.responseFromParts body headers clonedResponse.status clonedResponse.statusText)
                                                    )
                                        )
                        )
            )


stripParams : String -> List String -> Io w String
stripParams fullURL ignoreParams =
    Host.newUrl fullURL Nothing
        |> Io.andThen (\strippedURL -> Io.foldl (\param url -> Host.searchParamDelete url param) strippedURL ignoreParams)
        |> Io.map .href


{-| `cacheMatchIgnoreParams(cache, request, ignoreParams)`: an ordinary match when the request has
none of the parameters, else the first key that is the request once both are stripped.
-}
cacheMatchIgnoreParams : Cache -> Request -> List String -> Io w (Maybe Response)
cacheMatchIgnoreParams cache request ignoreParams =
    stripParams request.url ignoreParams
        |> Io.andThen
            (\strippedRequestURL ->
                if request.url == strippedRequestURL then
                    Host.cacheMatch cache request

                else
                    let
                        firstMatching keys =
                            case keys of
                                [] ->
                                    Io.succeed Nothing

                                cacheKey :: rest ->
                                    stripParams cacheKey.url ignoreParams
                                        |> Io.andThen
                                            (\strippedKeyURL ->
                                                if strippedRequestURL == strippedKeyURL then
                                                    Host.cacheMatch cache cacheKey

                                                else
                                                    firstMatching rest
                                            )
                    in
                    Host.cacheKeysIgnoringSearch cache request |> Io.andThen firstMatching
            )
