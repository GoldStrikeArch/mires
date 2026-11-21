module RetryTiming exposing (TimingHeader, calculateRetryTimingDelay, getRetryTimingHeader)

{-| Ky 2.2.0 `core/retry-timing.js`: which header gives a server-provided retry delay, and that
delay. Ky matches the three HTTP-date forms with anchored regular expressions of ASCII literals
and `\d`, each of a fixed length once the weekday is known; elm/core has no regular expressions,
so each form is checked field by field, at the positions its pattern fixes.
-}

import Host exposing (Headers, Io(..), andThen, map)


type alias TimingHeader =
    { value : String
    , allowTimestamp : Bool
    }


{-| A reset value is only read as an epoch from this second onwards (2001-09-09).
-}
timestampThresholdSeconds : Float
timestampThresholdSeconds =
    1000000000


msPerDay : Float
msPerDay =
    86400000


months : List String
months =
    [ "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" ]


weekdays : List String
weekdays =
    [ "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" ]


weekdayNames : List String
weekdayNames =
    [ "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday" ]


{-| Retry-After is authoritative when present. Only missing Retry-After falls through to the
rate-limit headers: RateLimit-Reset, Symfony's X-RateLimit-Retry-After, then GitHub's and
Twitter's reset headers.
-}
timingHeaders : List ( String, Bool )
timingHeaders =
    [ ( "Retry-After", False )
    , ( "RateLimit-Reset", True )
    , ( "X-RateLimit-Retry-After", False )
    , ( "X-RateLimit-Reset", True )
    , ( "X-Rate-Limit-Reset", True )
    ]


getRetryTimingHeader : Headers -> Io s (Maybe TimingHeader)
getRetryTimingHeader headers =
    let
        first candidates =
            case candidates of
                [] ->
                    Done Nothing

                ( name, allowTimestamp ) :: rest ->
                    Host.getHeader headers name
                        |> andThen
                            (\value ->
                                case value of
                                    Just text ->
                                        Done (Just { value = text, allowTimestamp = allowTimestamp })

                                    Nothing ->
                                        first rest
                            )
    in
    first timingHeaders


{-| `/^\d+$/`: one or more ASCII digits and nothing else. `Char.isDigit` is ASCII only.
-}
isDigits : String -> Bool
isDigits text =
    text /= "" && String.all Char.isDigit text


{-| A field of a pattern: the code units from one position to another, and what they must be.
-}
type alias Field =
    ( Int, Int, String -> Bool )


fits : List Field -> String -> Bool
fits fields value =
    List.all (\( from, to, test ) -> test (String.slice from to value)) fields


is : String -> String -> Bool
is =
    (==)


isMonth : String -> Bool
isMonth text =
    List.member text months


number : Int -> Int -> String -> Int
number from to value =
    String.slice from to value |> String.trim |> String.toInt |> Maybe.withDefault 0


monthIndex : String -> Int
monthIndex name =
    List.indexedMap Tuple.pair months
        |> List.filter (\( _, known ) -> known == name)
        |> List.head
        |> Maybe.map Tuple.first
        |> Maybe.withDefault -1


isLeapYear : Int -> Bool
isLeapYear year =
    (modBy 4 year == 0 && modBy 100 year /= 0) || modBy 400 year == 0


daysInMonth : Int -> Int -> Int
daysInMonth year month =
    case month of
        1 ->
            if isLeapYear year then
                29

            else
                28

        3 ->
            30

        5 ->
            30

        8 ->
            30

        10 ->
            30

        _ ->
            31


{-| Days from 1970-01-01 to a proleptic Gregorian date, as ECMAScript's MakeDay counts them. A day
past the month's end carries into the next month, as MakeDay does.
-}
daysFromCivil : Int -> Int -> Int -> Int
daysFromCivil year index day =
    let
        month =
            index + 1

        y =
            if month <= 2 then
                year - 1

            else
                year

        era =
            (if y >= 0 then
                y

             else
                y - 399
            )
                // 400

        yearOfEra =
            y - era * 400

        dayOfYear =
            (153 * modBy 12 (month + 9) + 2) // 5 + day - 1

        dayOfEra =
            yearOfEra * 365 + yearOfEra // 4 - yearOfEra // 100 + dayOfYear
    in
    era * 146097 + dayOfEra - 719468


{-| The year, month index and day of a day count, as getUTCFullYear and its siblings give them.
-}
civilFromDays : Int -> ( Int, Int, Int )
civilFromDays dayCount =
    let
        days =
            dayCount + 719468

        era =
            (if days >= 0 then
                days

             else
                days - 146096
            )
                // 146097

        dayOfEra =
            days - era * 146097

        yearOfEra =
            (dayOfEra - dayOfEra // 1460 + dayOfEra // 36524 - dayOfEra // 146096) // 365

        dayOfYear =
            dayOfEra - (365 * yearOfEra + yearOfEra // 4 - yearOfEra // 100)

        shiftedMonth =
            (5 * dayOfYear + 2) // 153

        month =
            if shiftedMonth < 10 then
                shiftedMonth + 2

            else
                shiftedMonth - 10

        carry =
            if month <= 1 then
                1

            else
                0
    in
    ( yearOfEra + era * 400 + carry, month, dayOfYear - (153 * shiftedMonth + 2) // 5 + 1 )


type alias Parts =
    { year : Int, month : Int, day : Int, hours : Int, minutes : Int, seconds : Int }


{-| Ky's createTimestamp: Date.UTC of the parts, with the year set again by setUTCFullYear, and
undefined unless every part round-trips. Date.UTC reads a year from 0 to 99 as 1900 + year, so
the day has to exist in that year too.
-}
createTimestamp : Parts -> Maybe Float
createTimestamp parts =
    let
        utcYear =
            if parts.year >= 0 && parts.year <= 99 then
                parts.year + 1900

            else
                parts.year
    in
    if parts.month == -1 || parts.hours > 23 || parts.minutes > 59 || parts.seconds > 60 then
        Nothing

    else if parts.day < 1 || parts.day > daysInMonth utcYear parts.month || parts.day > daysInMonth parts.year parts.month then
        Nothing

    else
        let
            timestamp =
                toFloat (daysFromCivil parts.year parts.month parts.day)
                    * msPerDay
                    + toFloat (parts.hours * 3600000 + parts.minutes * 60000 + min parts.seconds 59 * 1000)
        in
        -- A leap second is represented by parsing 60 as 59 and adding one second.
        if parts.seconds == 60 then
            Just (timestamp + 1000)

        else
            Just timestamp


{-| `HH:MM:SS` from a position.
-}
timeFields : Int -> List Field
timeFields at =
    [ ( at, at + 2, isDigits ), ( at + 2, at + 3, is ":" ), ( at + 3, at + 5, isDigits ), ( at + 5, at + 6, is ":" ), ( at + 6, at + 8, isDigits ) ]


{-| IMF-fixdate, `Sun, 06 Nov 1994 08:49:37 GMT`.
-}
imfDate : String -> Maybe Parts
imfDate value =
    if
        String.length value
            == 29
            && fits
                ([ ( 0, 3, \text -> List.member text weekdays ), ( 3, 5, is ", " ), ( 5, 7, isDigits ), ( 7, 8, is " " ), ( 8, 11, isMonth ), ( 11, 12, is " " ), ( 12, 16, isDigits ), ( 16, 17, is " " ) ]
                    ++ timeFields 17
                    ++ [ ( 25, 29, is " GMT" ) ]
                )
                value
    then
        Just
            { year = number 12 16 value
            , month = monthIndex (String.slice 8 11 value)
            , day = number 5 7 value
            , hours = number 17 19 value
            , minutes = number 20 22 value
            , seconds = number 23 25 value
            }

    else
        Nothing


{-| RFC 850, `Sunday, 06-Nov-94 08:49:37 GMT`: the full weekday name, then fixed fields.
-}
rfc850Date : String -> Maybe ( Parts, Int )
rfc850Date value =
    weekdayNames
        |> List.filter (\name -> String.startsWith name value)
        |> List.head
        |> Maybe.andThen
            (\name ->
                let
                    rest =
                        String.dropLeft (String.length name) value
                in
                if
                    String.length rest
                        == 24
                        && fits
                            ([ ( 0, 2, is ", " ), ( 2, 4, isDigits ), ( 4, 5, is "-" ), ( 5, 8, isMonth ), ( 8, 9, is "-" ), ( 9, 11, isDigits ), ( 11, 12, is " " ) ]
                                ++ timeFields 12
                                ++ [ ( 20, 24, is " GMT" ) ]
                            )
                            rest
                then
                    Just
                        ( { year = 0
                          , month = monthIndex (String.slice 5 8 rest)
                          , day = number 2 4 rest
                          , hours = number 12 14 rest
                          , minutes = number 15 17 rest
                          , seconds = number 18 20 rest
                          }
                        , number 9 11 rest
                        )

                else
                    Nothing
            )


{-| asctime, `Sun Nov  6 08:49:37 1994`: the day is two digits, or a space and one digit.
-}
asctimeDate : String -> Maybe Parts
asctimeDate value =
    let
        isDay text =
            isDigits text || (String.left 1 text == " " && isDigits (String.dropLeft 1 text))
    in
    if
        String.length value
            == 24
            && fits
                ([ ( 0, 3, \text -> List.member text weekdays ), ( 3, 4, is " " ), ( 4, 7, isMonth ), ( 7, 8, is " " ), ( 8, 10, isDay ), ( 10, 11, is " " ) ]
                    ++ timeFields 11
                    ++ [ ( 19, 20, is " " ), ( 20, 24, isDigits ) ]
                )
                value
    then
        Just
            { year = number 20 24 value
            , month = monthIndex (String.slice 4 7 value)
            , day = number 8 10 value
            , hours = number 11 13 value
            , minutes = number 14 16 value
            , seconds = number 17 19 value
            }

    else
        Nothing


{-| Ky's parseDate: the first of the three forms whose pattern matches decides. RFC 9110 maps a
two-digit year to the latest matching century that is not more than 50 years in the future.
-}
parseDate : String -> Io s (Maybe Float)
parseDate value =
    case imfDate value of
        Just parts ->
            Done (createTimestamp parts)

        Nothing ->
            case rfc850Date value of
                Just ( parts, twoDigitYear ) ->
                    Host.now
                        |> map
                            (\now ->
                                let
                                    today =
                                        floor (now / msPerDay)

                                    ( currentYear, currentMonth, currentDay ) =
                                        civilFromDays today

                                    currentCenturyYear =
                                        currentYear // 100 * 100 + twoDigitYear

                                    fiftyYearsFromNow =
                                        toFloat (daysFromCivil (currentYear + 50) currentMonth currentDay) * msPerDay + (now - toFloat today * msPerDay)
                                in
                                [ currentCenturyYear - 100, currentCenturyYear, currentCenturyYear + 100 ]
                                    |> List.foldl
                                        (\year timestamp ->
                                            case createTimestamp { parts | year = year } of
                                                Just candidate ->
                                                    if candidate <= fiftyYearsFromNow then
                                                        Just candidate

                                                    else
                                                        timestamp

                                                Nothing ->
                                                    timestamp
                                        )
                                        Nothing
                            )

                Nothing ->
                    Done (asctimeDate value |> Maybe.andThen createTimestamp)


calculateRetryTimingDelay : TimingHeader -> Io s (Maybe Float)
calculateRetryTimingDelay header =
    if isDigits header.value then
        let
            -- Number(value): Elm's String.toFloat is JS `+` once whitespace and x, b, o are ruled out.
            seconds =
                String.toFloat header.value |> Maybe.withDefault 0
        in
        -- Standard Retry-After numbers are delay seconds. Timestamp compatibility only applies to
        -- reset headers.
        if header.allowTimestamp && seconds >= timestampThresholdSeconds then
            Host.now |> map (\now -> Just (max 0 (seconds * 1000 - now)))

        else
            Done (Just (max 0 (seconds * 1000)))

    else
        -- createTimestamp only returns valid dates, so the delay is always finite.
        parseDate header.value
            |> andThen
                (\timestamp ->
                    case timestamp of
                        Just time ->
                            Host.now |> map (\now -> Just (max 0 (time - now)))

                        Nothing ->
                            Done Nothing
                )
