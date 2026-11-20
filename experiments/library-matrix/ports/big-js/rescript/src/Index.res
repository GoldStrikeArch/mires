type operand
type operation={op:string,x:operand,y?:operand,dp?:float,rm?:float,n?:float,mode?:float}
type input={cases:array<operation>,repeat?:int}
@val external text:operand=>string="String"
let integer=(value, fallback)=>switch value {
  | None=>fallback
  | Some(f)=>let n=Float.toInt(f); Int.toFloat(n)==f && n != -2000002 ? n : -2000001
}
let run=(input:input)=>{
  let lines=[]
  for _i in 1 to input.repeat->Option.getOr(1) {
    input.cases->Array.forEach(c=>{
      let rm=integer(c.rm,1)
      let result=try {
        Decimal.operate(c.op,text(c.x),c.y->Option.map(text)->Option.getOr(""),integer(c.n,-2000002),integer(c.dp,20),rm,integer(c.mode,rm))
      } catch { | Decimal.Invalid(s)=>s }
      lines->Array.push(result)
    })
  }
  lines->Array.join("\n")
}
