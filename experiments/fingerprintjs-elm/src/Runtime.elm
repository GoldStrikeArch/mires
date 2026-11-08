module Runtime exposing (main)

{-| Empty worker used only to size the Elm 0.19 runtime for attribution.json.
-}

import Platform


main : Program () () ()
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \_ model -> ( model, Cmd.none )
        , subscriptions = \_ -> Sub.none
        }
