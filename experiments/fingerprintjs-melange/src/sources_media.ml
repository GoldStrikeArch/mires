open Host

let does_match (query : string) : bool =
  to_bool (get (match_media query) "matches")

let get_color_gamut (_ : js) : js =
  if does_match "(color-gamut: rec2020)" then of_string "rec2020"
  else if does_match "(color-gamut: p3)" then of_string "p3"
  else if does_match "(color-gamut: srgb)" then of_string "srgb"
  else undefined

let are_colors_inverted (_ : js) : js =
  if does_match "(inverted-colors: inverted)" then of_bool true
  else if does_match "(inverted-colors: none)" then of_bool false
  else undefined

let are_colors_forced (_ : js) : js =
  if does_match "(forced-colors: active)" then of_bool true
  else if does_match "(forced-colors: none)" then of_bool false
  else undefined

let get_monochrome_depth (_ : js) : js =
  if not (does_match "(min-monochrome: 0)") then undefined
  else
    let rec loop i =
      if i > 100 then
        let throw : string -> js =
          [%mel.raw {| function(m) { throw new Error(m); } |}]
        in
        throw "Too high value"
      else if does_match ("(max-monochrome: " ^ to_string_radix i 10 ^ ")") then
        of_int i
      else loop (i + 1)
    in
    loop 0

let get_contrast_preference (_ : js) : js =
  if does_match "(prefers-contrast: no-preference)" then of_int 0
  else if
    does_match "(prefers-contrast: high)" || does_match "(prefers-contrast: more)"
  then of_int 1
  else if
    does_match "(prefers-contrast: low)" || does_match "(prefers-contrast: less)"
  then of_int (-1)
  else if does_match "(prefers-contrast: forced)" then of_int 10
  else undefined

let is_motion_reduced (_ : js) : js =
  if does_match "(prefers-reduced-motion: reduce)" then of_bool true
  else if does_match "(prefers-reduced-motion: no-preference)" then of_bool false
  else undefined

let is_transparency_reduced (_ : js) : js =
  if does_match "(prefers-reduced-transparency: reduce)" then of_bool true
  else if does_match "(prefers-reduced-transparency: no-preference)" then
    of_bool false
  else undefined

let is_hdr (_ : js) : js =
  if does_match "(dynamic-range: high)" then of_bool true
  else if does_match "(dynamic-range: standard)" then of_bool false
  else undefined
