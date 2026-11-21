(* Ky 2.2.0 core/retry-timing.js: which header gives a server-provided retry delay, and that
   delay. The patterns are Ky's own, run by the host's RegExp. *)

type header = { value : string; allow_timestamp : bool }

(* A reset value is only read as an epoch from this second onwards (2001-09-09). *)
let timestamp_threshold_seconds = 1_000_000_000.
let ms_per_day = 86_400_000.
let delay_pattern = Host.regexp {|^\d+$|} ""
let months = [| "Jan"; "Feb"; "Mar"; "Apr"; "May"; "Jun"; "Jul"; "Aug"; "Sep"; "Oct"; "Nov"; "Dec" |]

(* HTTP-date parsing is limited to IMF-fixdate and the two obsolete HTTP-date formats. *)
let imf_date_pattern =
  Host.regexp
    {|^(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun), (\d{2}) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{4}) (\d{2}):(\d{2}):(\d{2}) GMT$|}
    ""

let rfc850_date_pattern =
  Host.regexp
    {|^(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday), (\d{2})-(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)-(\d{2}) (\d{2}):(\d{2}):(\d{2}) GMT$|}
    ""

let asctime_date_pattern =
  Host.regexp
    {|^(Mon|Tue|Wed|Thu|Fri|Sat|Sun) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{2}| \d) (\d{2}:\d{2}:\d{2}) (\d{4})$|}
    ""

(* Retry-After is authoritative when present. Only missing Retry-After falls through to the
   rate-limit headers: RateLimit-Reset, Symfony's X-RateLimit-Retry-After, then GitHub's and
   Twitter's reset headers. *)
let timing_headers =
  [
    ("Retry-After", false);
    ("RateLimit-Reset", true);
    ("X-RateLimit-Retry-After", false);
    ("X-RateLimit-Reset", true);
    ("X-Rate-Limit-Reset", true);
  ]

let get_retry_timing_header headers =
  List.find_map
    (fun (name, allow_timestamp) ->
      Option.map (fun value -> { value; allow_timestamp }) (Host.header headers name))
    timing_headers

let is_leap_year year = (year mod 4 = 0 && year mod 100 <> 0) || year mod 400 = 0

let days_in_month year month =
  match month with
  | 1 -> if is_leap_year year then 29 else 28
  | 3 | 5 | 8 | 10 -> 30
  | _ -> 31

(* Days from 1970-01-01 to a proleptic Gregorian date, as ECMAScript's MakeDay counts them. A day
   past the month's end carries into the next month, as MakeDay does. *)
let days_from_civil year month day =
  let month = month + 1 in
  let year = if month <= 2 then year - 1 else year in
  let era = (if year >= 0 then year else year - 399) / 400 in
  let year_of_era = year - (era * 400) in
  let day_of_year = (((153 * ((month + 9) mod 12)) + 2) / 5) + day - 1 in
  let day_of_era = (year_of_era * 365) + (year_of_era / 4) - (year_of_era / 100) + day_of_year in
  (era * 146_097) + day_of_era - 719_468

(* The year, month index and day of a day count, as getUTCFullYear and its siblings give them. *)
let civil_from_days days =
  let days = days + 719_468 in
  let era = (if days >= 0 then days else days - 146_096) / 146_097 in
  let day_of_era = days - (era * 146_097) in
  let year_of_era =
    (day_of_era - (day_of_era / 1460) + (day_of_era / 36_524) - (day_of_era / 146_096)) / 365
  in
  let day_of_year = day_of_era - ((365 * year_of_era) + (year_of_era / 4) - (year_of_era / 100)) in
  let shifted_month = ((5 * day_of_year) + 2) / 153 in
  let day = day_of_year - (((153 * shifted_month) + 2) / 5) + 1 in
  let month = if shifted_month < 10 then shifted_month + 2 else shifted_month - 10 in
  (year_of_era + (era * 400) + (if month <= 1 then 1 else 0), month, day)

let number text = int_of_float (Host.number text)

(* Ky's createTimestamp: Date.UTC of the parts, with the year set again by setUTCFullYear, and
   undefined unless every part round-trips. Date.UTC reads a year from 0 to 99 as 1900 + year, so
   the day has to exist in that year too. *)
let create_timestamp ~year ~month ~day ~hours ~minutes ~seconds =
  match Array.find_index (String.equal month) months with
  | None -> None
  | Some month ->
      let day = number day and hours = number hours in
      let minutes = number minutes and seconds = number seconds in
      if hours > 23 || minutes > 59 || seconds > 60 then None
      else
        let normalized_seconds = min seconds 59 in
        let utc_year = if year >= 0 && year <= 99 then year + 1900 else year in
        if day < 1 || day > days_in_month utc_year month || day > days_in_month year month then None
        else
          let timestamp =
            (float_of_int (days_from_civil year month day) *. ms_per_day)
            +. float_of_int ((hours * 3_600_000) + (minutes * 60_000) + (normalized_seconds * 1000))
          in
          (* A leap second is represented by parsing 60 as 59 and adding one second. *)
          Some (if seconds = 60 then timestamp +. 1000. else timestamp)

let capture groups index = Option.value groups.(index) ~default:""

let parse_date value =
  match Host.exec imf_date_pattern value with
  | Some date ->
      create_timestamp
        ~year:(number (capture date 3))
        ~month:(capture date 2) ~day:(capture date 1) ~hours:(capture date 4)
        ~minutes:(capture date 5) ~seconds:(capture date 6)
  | None -> (
      match Host.exec rfc850_date_pattern value with
      | Some date ->
          let now = Host.now () in
          let today = int_of_float (floor (now /. ms_per_day)) in
          let current_year, current_month, current_day = civil_from_days today in
          let current_century_year = (current_year / 100 * 100) + number (capture date 3) in
          let fifty_years_from_now =
            (float_of_int (days_from_civil (current_year + 50) current_month current_day) *. ms_per_day)
            +. (now -. (float_of_int today *. ms_per_day))
          in
          (* RFC 9110 maps two-digit years to the latest matching century that is not more than
             50 years in the future. *)
          List.fold_left
            (fun timestamp year ->
              match
                create_timestamp ~year ~month:(capture date 2) ~day:(capture date 1)
                  ~hours:(capture date 4) ~minutes:(capture date 5) ~seconds:(capture date 6)
              with
              | Some candidate when candidate <= fifty_years_from_now -> Some candidate
              | _ -> timestamp)
            None
            [ current_century_year - 100; current_century_year; current_century_year + 100 ]
      | None -> (
          match Host.exec asctime_date_pattern value with
          | Some date -> (
              match String.split_on_char ':' (capture date 4) with
              | [ hours; minutes; seconds ] ->
                  create_timestamp
                    ~year:(number (capture date 5))
                    ~month:(capture date 2)
                    ~day:(Host.trim (capture date 3))
                    ~hours ~minutes ~seconds
              | _ -> None)
          | None -> None))

let calculate_retry_timing_delay { value; allow_timestamp } =
  if Host.test delay_pattern value then
    let seconds = Host.number value in
    (* Standard Retry-After numbers are delay seconds. Timestamp compatibility only applies to
       reset headers. *)
    let delay =
      if allow_timestamp && seconds >= timestamp_threshold_seconds then (seconds *. 1000.) -. Host.now ()
      else seconds *. 1000.
    in
    Some (Host.math_max 0. delay)
  else
    (* createTimestamp only returns valid dates, so the delay is always finite. *)
    Option.map (fun timestamp -> Host.math_max 0. (timestamp -. Host.now ())) (parse_date value)
