# runir 

Intermediate representation of type information associated to runmd defined objects.

## Design

To store and retrieve information, object data is interned and assigned an intern handle.

A representation consists of "representation" levels. A level is a set of "tags" which are all assigned to an intern handle which represents the level.

Each handle can be used to lookup it's value from an "intern table". The same handle can be used to lookup all tags that belong to respective level. Representations are constructed w/ a factory which can be used to merge handles into a single u64 value. This value is the tail of a linked list, and can be converted back into each individual level.

**Representation Linked List** 

The representation is stored as a single u64 which is the tail of a doublely-linked list.

```
// The following variables represent an intern handle
let ROOT;
let LEVEL_1;
let LEVEL_2;
let LEVEL_3;

// An intern handle is a u64 value split into a lhs and rhs

// A link is created by computing the XOR value of the prev/next node in the list
// Example: ROOT ^ LEVEL_1;

// This link is also an intern handle as well and is stored in the following format
let LINK = ROOT ^ LEVEL_1 | LEVEL_1

// The link is stored in an intern table and can be accessed by the rhs handle.

HANDLES.assign(LEVEL_1, LINK);

// The representation uses the tail in, so in this case the representation would be, 

let repr = Repr(LINK);

// To get the previous value take the xor of the rhs and lhs

let (prev, current) = (LINK.lhs ^ LINK.rhs, LINK.rhs);

// This procedure is repeated for each level. With the tail value, each level can be restored by looking up the interned link.
```

This is better illustrated by reading the tests written for the `Repr` type.

## Representation Levels 

The first 4 levels are accounted for by this crate. They include: `ResourceLevel`, `FieldLevel`, `NodeLevel`, and `HostLevel`. When a representation is built, each level must be added in order. For example, a representation cannot have a node level without also having a resource level and vice versa.

**`ResourceLevel`**   

This level contains tags that describe the type information of the resource.

**`FieldLevel`**      

This level contains tags that describe the relationship between the receiving/owning type of the resource.

**`NodeLevel`**       

This is the first dynamic level of representation and represents the input configuration of the node which initialized this resource.

**`HostLevel`**       

This is the next dynamic level of representation and contains information provided by the host that is managing the current resource node. 

**Defined Tags** - The following table are all currently defined tags.

| Level | LevelName     | Name          | Type          | Description |
| ----- | ------------- | ------------- | ------------- | ------------------------------------------------------------------------------------- |
| 0     | ResourceLevel | type_id       | TypeId        | The type id value assigned by the compiler/runtime.                                   |
| 0     | ResourceLevel | type_name     | &'static str  | The name of the type assigned by the compiler/runtime.                                |
| 0     | ResourceLevel | type_size     | usize         | The size of the type.                                                                 |
| 1     | FieldLevel    | owner_type_id | TypeId        | The type id value assigned by the compiler/runtime for the owner of this field.       |
| 1     | FieldLevel    | owner_name    | &'static str  | The type name value assigned by the compiler/runtime for the owner of this field.     |
| 1     | FieldLevel    | owner_size    | usize         | The size of the type for the owner of this field.                                     |
| 1     | FieldLevel    | field_offset  | usize         | The offset of this field according to the owner of this field.                        |
| 1     | FieldLevel    | field_name    | &'static str  | The name of the field according to the owner of this field.                           |
| 2     | NodeLevel     | input         | String        | The input value passed to the node that initialized this representation.              |
| 2     | NodeLevel     | tag           | String        | The tag value passed to the node that initialized this representation.                |
| 2     | NodeLevel     | idx           | usize         | The index or ordinal position of this node, with respect to it's parent node.         |
| 2     | NodeLevel     | annotations   | Map*          | Ordered map of annotations passed to the node that initialized this representation.   |
| 3     | HostLevel     | address       | String        | The address assigned to this representation managed by a host namespace.              |

***Map type is `BTreeMap<String, String>`**

Consumers of this library can defined their own level/tag which start at representation Level 3 or 4 depending on use case. 

The max allowed representation levels is 8 (including the root level).

# Roadmap
- **Phase 0** Integrate into reality.
- **Phase 1** Initialize intern tables from filesystem.
- **Phase 2** Bootstrap runir from runmd.
