//! The abstract heap and the place navigation both interpreters share.

use crate::jir::semantic::TypeId;
use crate::jir::types::{TypeKind, TypeTable};

use super::value::{HeapId, Location, Step, Value};
use super::Trap;

/// The heap: every local, cell, SSA value slot, and constant lives in one object.
#[derive(Clone, Debug, Default)]
pub struct Heap {
    objects: Vec<Value>,
    written: Vec<bool>,
}

impl Heap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self, value: Value) -> HeapId {
        let id = HeapId(self.objects.len() as u32);
        self.objects.push(value);
        self.written.push(false);
        id
    }

    /// Whether mutable access or storage reconstruction has occurred since allocation.
    /// Writing the same value or restoring an earlier value does not erase this history.
    pub fn was_written(&self, object: HeapId) -> bool {
        self.written
            .get(object.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    pub fn get(&self, location: &Location) -> Result<&Value, Trap> {
        let mut current = self
            .objects
            .get(location.heap.0 as usize)
            .ok_or(Trap::Ub("dangling pointer"))?;
        for step in &location.path {
            current = match (current, step) {
                (Value::Aggregate { fields, .. }, Step::Field(index)) => fields
                    .get(*index as usize)
                    .ok_or(Trap::Ub("field projection out of range"))?,
                (Value::Aggregate { fields, .. }, Step::Index(index)) => fields
                    .get(*index as usize)
                    .ok_or(Trap::Ub("index out of range"))?,
                (Value::Undef, _) => return Err(Trap::Ub("read of uninitialized storage")),
                _ => return Err(Trap::Ub("projection of a non-aggregate")),
            };
        }
        Ok(current)
    }

    /// Reads and clones the value at `location`.
    pub fn read(&self, location: &Location) -> Result<Value, Trap> {
        self.get(location).cloned()
    }

    /// The mutable slot at `location`. Storage that is `Undef` or holds the wrong variant on the
    /// way is created as an aggregate of the type the caller tracked (`types` per step); a caller
    /// that wants no creation passes `None`.
    pub fn slot(
        &mut self,
        location: &Location,
        created: Option<&[(TypeId, Option<u32>)]>,
        table: &TypeTable,
    ) -> Result<HeapSlot<'_>, Trap> {
        let mut current = self
            .objects
            .get_mut(location.heap.0 as usize)
            .ok_or(Trap::Ub("dangling pointer"))?;
        let written = &mut self.written[location.heap.0 as usize];
        for (position, step) in location.path.iter().enumerate() {
            if let Some(created) = created {
                let (ty, variant) = created
                    .get(position)
                    .copied()
                    .ok_or(Trap::Unsupported("place creation without a type"))?;
                let variant = variant.unwrap_or(0);
                let rebuild = match current {
                    Value::Aggregate {
                        variant: active, ..
                    } => *active != variant && matches!(step, Step::Field(_)),
                    _ => true,
                };
                if rebuild {
                    *current = fresh_aggregate(ty, variant, table)?;
                    *written = true;
                }
            }
            let index = match step {
                Step::Field(index) => *index as usize,
                Step::Index(index) => *index as usize,
            };
            current = match current {
                Value::Aggregate { fields, .. } => fields
                    .get_mut(index)
                    .ok_or(Trap::Ub("projection out of range"))?,
                Value::Undef => return Err(Trap::Ub("write into uninitialized storage")),
                _ => return Err(Trap::Ub("projection of a non-aggregate")),
            };
        }
        Ok(HeapSlot {
            value: current,
            written,
        })
    }

    pub fn write(&mut self, location: &Location, value: Value) -> Result<(), Trap> {
        *self.slot(location, None, &TypeTable::new())? = value;
        Ok(())
    }
}

/// A heap slot that records mutable access while allowing unmodified storage to be borrowed.
#[derive(Debug)]
pub struct HeapSlot<'a> {
    value: &'a mut Value,
    written: &'a mut bool,
}

impl std::ops::Deref for HeapSlot<'_> {
    type Target = Value;

    fn deref(&self) -> &Value {
        self.value
    }
}

impl std::ops::DerefMut for HeapSlot<'_> {
    fn deref_mut(&mut self) -> &mut Value {
        *self.written = true;
        self.value
    }
}

/// An aggregate of `ty` (variant `variant`) with every field `Undef`.
pub fn fresh_aggregate(ty: TypeId, variant: u32, table: &TypeTable) -> Result<Value, Trap> {
    let count = field_count(ty, variant, table).ok_or(Trap::Unsupported(
        "creating storage for a type without a known field layout",
    ))?;
    Ok(Value::Aggregate {
        ty,
        variant,
        fields: vec![Value::Undef; count],
    })
}

/// How many fields (or elements) an aggregate of `ty` in `variant` has.
pub fn field_count(ty: TypeId, variant: u32, table: &TypeTable) -> Option<usize> {
    let descriptor = table.get(ty)?;
    match &descriptor.kind {
        TypeKind::Adt { variants, .. } => Some(variants.get(variant as usize)?.fields.len()),
        TypeKind::Array { length, .. } => Some(*length as usize),
        TypeKind::Tuple(elements) => Some(elements.len()),
        TypeKind::Closure { upvars, .. } => Some(upvars.len()),
        _ => None,
    }
}

/// The type of field `field` of `ty` (downcast to `variant`), or the element type.
pub fn field_type(
    ty: TypeId,
    variant: Option<u32>,
    field: u32,
    table: &TypeTable,
) -> Option<TypeId> {
    let descriptor = table.get(ty)?;
    match &descriptor.kind {
        TypeKind::Array { element, .. } => Some(*element),
        TypeKind::Slice(element) => Some(*element),
        TypeKind::Adt { variants, .. } => Some(
            variants
                .get(variant.unwrap_or(0) as usize)?
                .fields
                .get(field as usize)?
                .ty,
        ),
        TypeKind::Tuple(elements) => elements.get(field as usize).copied(),
        TypeKind::Closure { upvars, .. } => upvars.get(field as usize).copied(),
        _ => None,
    }
}

/// The element type of an array or slice.
pub fn element_type(ty: TypeId, table: &TypeTable) -> Option<TypeId> {
    match &table.get(ty)?.kind {
        TypeKind::Array { element, .. } | TypeKind::Slice(element) => Some(*element),
        _ => None,
    }
}

/// The pointee of a reference, raw pointer, or a wrapper that peels to one (`Box`).
pub fn pointee_type(ty: TypeId, table: &TypeTable) -> Option<TypeId> {
    let peeled = table.peeled(ty);
    match &table.get(peeled)?.kind {
        TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
            Some(*pointee)
        }
        _ => None,
    }
}

/// A place under evaluation: one location, a slice window over an array, or string data.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaceRef {
    Loc(Location),
    Slice { buf: Location, off: u64, len: u64 },
    Str(String),
}

/// A place path step, evaluated (index operands already read).
#[derive(Clone, Copy, Debug)]
pub enum PathStep {
    Deref,
    Field {
        field: u32,
    },
    Index(u64),
    ConstantIndex {
        offset: u64,
        min_length: u64,
        from_end: bool,
    },
    Subslice {
        from: u64,
        to: u64,
        from_end: bool,
    },
    Downcast {
        variant: u32,
    },
}

/// The result of navigating a path: the place, its type, and the `(type, variant)` of the
/// storage each `Location` path step passed through (for creation on write).
#[derive(Clone, Debug)]
pub struct Navigated {
    pub place: PlaceRef,
    pub ty: TypeId,
    pub created: Vec<(TypeId, Option<u32>)>,
}

impl Heap {
    /// The length of the array or slice at `place`.
    fn len_of(&self, place: &PlaceRef) -> Result<u64, Trap> {
        match place {
            PlaceRef::Slice { len, .. } => Ok(*len),
            PlaceRef::Str(s) => Ok(s.len() as u64),
            PlaceRef::Loc(location) => match self.get(location)? {
                Value::Aggregate { fields, .. } => Ok(fields.len() as u64),
                _ => Err(Trap::Ub("length of a non-array place")),
            },
        }
    }

    /// Follows the pointer stored at `place`.
    fn deref(&self, place: &PlaceRef) -> Result<PlaceRef, Trap> {
        let PlaceRef::Loc(location) = place else {
            return Err(Trap::Ub("deref of a slice or str place"));
        };
        let mut value = self.get(location)?;
        // A `Box` or other transparent wrapper: peel single-field aggregates down to the pointer.
        loop {
            match value {
                Value::Aggregate { fields, .. } if fields.len() == 1 => value = &fields[0],
                _ => break,
            }
        }
        match value {
            Value::Ptr(target) => Ok(PlaceRef::Loc(target.clone())),
            Value::Slice { buf, off, len } => Ok(PlaceRef::Slice {
                buf: buf.clone(),
                off: *off,
                len: *len,
            }),
            Value::Str(s) => Ok(PlaceRef::Str(s.clone())),
            Value::Undef => Err(Trap::Ub("deref of uninitialized pointer")),
            _ => Err(Trap::Unsupported("deref of a non-pointer value")),
        }
    }

    /// Navigates `steps` from `start` (of type `ty`). A `strict` navigation (a read) requires every
    /// downcast to name the active variant; a lenient one (a write or borrow) lets the caller
    /// create the storage.
    pub fn navigate(
        &self,
        start: PlaceRef,
        ty: TypeId,
        steps: impl IntoIterator<Item = PathStep>,
        table: &TypeTable,
        strict: bool,
    ) -> Result<Navigated, Trap> {
        let mut place = start;
        let mut ty = ty;
        let mut variant: Option<u32> = None;
        let mut created = Vec::new();
        for step in steps {
            match step {
                PathStep::Deref => {
                    let pointee = pointee_type(ty, table)
                        .ok_or(Trap::Unsupported("deref of a non-pointer type"))?;
                    place = self.deref(&place)?;
                    ty = pointee;
                    variant = None;
                    created.clear();
                }
                PathStep::Field { field } => {
                    let field_ty = field_type(ty, variant, field, table)
                        .ok_or(Trap::Unsupported("field of a type without fields"))?;
                    let PlaceRef::Loc(location) = &place else {
                        return Err(Trap::Ub("field of a slice place"));
                    };
                    created.push((ty, variant));
                    place = PlaceRef::Loc(location.child(Step::Field(field)));
                    ty = field_ty;
                    variant = None;
                }
                PathStep::Downcast { variant: v } => {
                    if strict {
                        let PlaceRef::Loc(location) = &place else {
                            return Err(Trap::Ub("downcast of a slice place"));
                        };
                        match self.get(location)? {
                            Value::Aggregate {
                                variant: active, ..
                            } if *active == v => {}
                            Value::Aggregate { .. } => {
                                return Err(Trap::Ub("downcast to an inactive variant"))
                            }
                            _ => return Err(Trap::Ub("downcast of a non-aggregate")),
                        }
                    }
                    variant = Some(v);
                }
                PathStep::Index(index) => {
                    let element = element_type(ty, table)
                        .ok_or(Trap::Unsupported("index of a non-array type"))?;
                    let len = self.len_of(&place)?;
                    if index >= len {
                        return Err(Trap::Ub("index out of bounds"));
                    }
                    place = self.element(&place, index)?;
                    created.push((ty, None));
                    ty = element;
                    variant = None;
                }
                PathStep::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                } => {
                    let element = element_type(ty, table)
                        .ok_or(Trap::Unsupported("index of a non-array type"))?;
                    let len = self.len_of(&place)?;
                    if len < min_length {
                        return Err(Trap::Ub("constant index below the minimum length"));
                    }
                    let index = if from_end { len - offset } else { offset };
                    place = self.element(&place, index)?;
                    created.push((ty, None));
                    ty = element;
                    variant = None;
                }
                PathStep::Subslice { from, to, from_end } => {
                    let len = self.len_of(&place)?;
                    let end = if from_end { len - to } else { to };
                    if from > end || end > len {
                        return Err(Trap::Ub("subslice out of bounds"));
                    }
                    place = match place {
                        PlaceRef::Loc(buf) => PlaceRef::Slice {
                            buf,
                            off: from,
                            len: end - from,
                        },
                        PlaceRef::Slice { buf, off, .. } => PlaceRef::Slice {
                            buf,
                            off: off + from,
                            len: end - from,
                        },
                        PlaceRef::Str(s) => {
                            PlaceRef::Str(s[from as usize..end as usize].to_owned())
                        }
                    };
                    // The type of a subslice of an array is the slice type, which the projection
                    // in JIR-S records as the place's final type; keep the element type reachable.
                    let element = element_type(ty, table)
                        .ok_or(Trap::Unsupported("subslice of a non-array type"))?;
                    ty = slice_type_of(element, ty, table);
                    variant = None;
                    created.clear();
                }
            }
        }
        Ok(Navigated { place, ty, created })
    }

    fn element(&self, place: &PlaceRef, index: u64) -> Result<PlaceRef, Trap> {
        match place {
            PlaceRef::Loc(location) => Ok(PlaceRef::Loc(location.child(Step::Index(index)))),
            PlaceRef::Slice { buf, off, .. } => {
                Ok(PlaceRef::Loc(buf.child(Step::Index(off + index))))
            }
            PlaceRef::Str(_) => Err(Trap::Unsupported("byte index into str data")),
        }
    }

    /// Reads a place as a value.
    pub fn read_place(&self, place: &PlaceRef) -> Result<Value, Trap> {
        match place {
            PlaceRef::Loc(location) => self.read(location),
            PlaceRef::Slice { .. } | PlaceRef::Str(_) => {
                Err(Trap::Ub("read of an unsized place by value"))
            }
        }
    }

    /// The pointer value that references `place`.
    pub fn address_of(place: &PlaceRef) -> Value {
        match place {
            PlaceRef::Loc(location) => Value::Ptr(location.clone()),
            PlaceRef::Slice { buf, off, len } => Value::Slice {
                buf: buf.clone(),
                off: *off,
                len: *len,
            },
            PlaceRef::Str(s) => Value::Str(s.clone()),
        }
    }

    /// The place a pointer value references.
    pub fn place_of(pointer: &Value) -> Result<PlaceRef, Trap> {
        match pointer {
            Value::Ptr(location) => Ok(PlaceRef::Loc(location.clone())),
            Value::Slice { buf, off, len } => Ok(PlaceRef::Slice {
                buf: buf.clone(),
                off: *off,
                len: *len,
            }),
            Value::Str(s) => Ok(PlaceRef::Str(s.clone())),
            _ => Err(Trap::Unsupported("place of a non-pointer value")),
        }
    }
}

/// The slice type over `element` if the table has one, else the array type itself (the interpreter
/// only needs the element type reachable through it).
fn slice_type_of(element: TypeId, fallback: TypeId, table: &TypeTable) -> TypeId {
    table
        .descriptors()
        .iter()
        .find(|descriptor| matches!(descriptor.kind, TypeKind::Slice(e) if e == element))
        .map_or(fallback, |descriptor| descriptor.id)
}
