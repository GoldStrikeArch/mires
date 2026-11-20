export function variant(input,k) {
  return {...input,batches:input.batches.map(batch=>({...batch,rows:batch.rows.map(row=>({...row,value:row.value+'-'+(k>>>0)}))}))}
}
