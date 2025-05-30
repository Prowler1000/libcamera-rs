use std::{
    marker::PhantomData,
    ptr::{addr_of_mut, null, NonNull},
};

use libc::free;
use libcamera_sys::*;
use thiserror::Error;

use crate::{
    control_value::{ControlValue, ControlValueError},
    controls::{self, ControlId},
    properties::{self, PropertyId},
    utils::{UniquePtr, UniquePtrTarget},
};

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("Control id {0} not found")]
    NotFound(u32),
    #[error("Control value error: {0}")]
    ValueError(#[from] ControlValueError),
}

pub trait ControlEntry:
    Clone + Into<ControlValue> + TryFrom<ControlValue, Error = ControlValueError> + core::fmt::Debug
{
    const ID: u32;
}

pub trait Control: ControlEntry {}
pub trait Property: ControlEntry {}

/// Dynamic Control, which does not have strong typing.
pub trait DynControlEntry: core::fmt::Debug {
    fn id(&self) -> u32;
    fn value(&self) -> ControlValue;
}

impl<T: ControlEntry> DynControlEntry for T {
    fn id(&self) -> u32 {
        Self::ID
    }

    fn value(&self) -> ControlValue {
        self.clone().into()
    }
}

#[repr(transparent)]
pub struct ControlInfo(libcamera_control_info_t);

impl ControlInfo {
    pub(crate) unsafe fn from_ptr<'a>(ptr: NonNull<libcamera_control_info_t>) -> &'a mut Self {
        // Safety: we can cast it because of `#[repr(transparent)]`
        &mut *(ptr.as_ptr() as *mut Self)
    }
    pub(crate) fn ptr(&self) -> *const libcamera_control_info_t {
        // Safety: we can cast it because of `#[repr(transparent)]`
        &self.0 as *const libcamera_control_info_t
    }

    pub fn min(&self) -> Result<ControlValue, ControlValueError> {
        let value_ptr = unsafe { libcamera_control_info_min(self.ptr()) };
        let tmp = unsafe { ControlValue::read(NonNull::new(value_ptr as _).unwrap()) };
        unsafe {
            libcamera_control_value_destroy(value_ptr as _);
        }
        tmp
    }
    pub fn max(&self) -> Result<ControlValue, ControlValueError> {
        let value_ptr = unsafe { libcamera_control_info_max(self.ptr()) };
        let tmp = unsafe { ControlValue::read(NonNull::new(value_ptr as _).unwrap()) };
        unsafe {
            libcamera_control_value_destroy(value_ptr as _);
        }
        tmp
    }
    pub fn def(&self) -> Result<ControlValue, ControlValueError> {
        let value_ptr = unsafe { libcamera_control_info_def(self.ptr()) };
        let tmp = unsafe { ControlValue::read(NonNull::new(value_ptr as _).unwrap()) };
        unsafe {
            libcamera_control_value_destroy(value_ptr as _);
        }
        tmp
    }
    pub fn values(&self) -> Result<Vec<ControlValue>, ControlValueError> {
        let mut size = 0;
        let values_ptr = unsafe {
            let ptr_to_array = libcamera_control_info_values(self.ptr(), addr_of_mut!(size));
            std::slice::from_raw_parts_mut(ptr_to_array as *mut _, size)
        };
        let values = values_ptr
            .iter_mut()
            .map(|value| -> Result<ControlValue, ControlValueError> {
                unsafe { ControlValue::read(NonNull::new(&mut *value as *mut _).unwrap()) }
            })
            .collect::<Result<Vec<_>, _>>();
        unsafe {
            free(values_ptr.as_mut_ptr() as *mut _);
        }
        values
    }
}

impl std::fmt::Debug for ControlInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlInfo")
            .field("min", &self.min())
            .field("max", &self.max())
            .field("def", &self.def())
            .field("values", &self.values())
            .finish()
    }
}

#[repr(transparent)]
pub struct ControlInfoMap(libcamera_control_info_map_t);

impl ControlInfoMap {
    pub(crate) unsafe fn from_ptr<'a>(ptr: NonNull<libcamera_control_info_map_t>) -> &'a mut Self {
        // Safety: we can cast it because of `#[repr(transparent)]`
        &mut *(ptr.as_ptr() as *mut Self)
    }

    pub(crate) fn ptr(&self) -> *const libcamera_control_info_map_t {
        // Safety: we can cast it because of `#[repr(transparent)]`
        &self.0 as *const libcamera_control_info_map_t
    }

    pub fn get(&self, control: u32) -> Option<&ControlInfo> {
        let info_ptr = unsafe { libcamera_control_info_map_get(self.ptr(), control) };
        if info_ptr.is_null() {
            None
        } else {
            unsafe { Some(ControlInfo::from_ptr(NonNull::new_unchecked(info_ptr as *mut _))) }
        }
    }
}

#[repr(transparent)]
pub struct ControlList(libcamera_control_list_t);

impl UniquePtrTarget for ControlList {
    unsafe fn ptr_new() -> *mut Self {
        libcamera_control_list_create() as *mut Self
    }

    unsafe fn ptr_drop(ptr: *mut Self) {
        libcamera_control_list_destroy(ptr as *mut libcamera_control_list_t)
    }
}

impl ControlList {
    pub fn new() -> UniquePtr<Self> {
        UniquePtr::new()
    }

    pub(crate) unsafe fn from_ptr<'a>(ptr: NonNull<libcamera_control_list_t>) -> &'a mut Self {
        // Safety: we can cast it because of `#[repr(transparent)]`
        &mut *(ptr.as_ptr() as *mut Self)
    }

    pub(crate) fn ptr(&self) -> *const libcamera_control_list_t {
        // Safety: we can cast it because of `#[repr(transparent)]`
        &self.0 as *const libcamera_control_list_t
    }

    pub fn get<C: Control>(&self) -> Result<C, ControlError> {
        let val_ptr = NonNull::new(unsafe { libcamera_control_list_get(self.ptr().cast_mut(), C::ID as _).cast_mut() })
            .ok_or(ControlError::NotFound(C::ID))?;

        let val = unsafe { ControlValue::read(val_ptr) }?;
        Ok(C::try_from(val)?)
    }

    /// Sets control value.
    ///
    /// This can fail if control is not supported by the camera, but due to libcamera API limitations an error will not
    /// be returned. Use [ControlList::get] if you need to ensure that value was set.
    pub fn set<C: Control>(&mut self, val: C) -> Result<(), ControlError> {
        let ctrl_val: ControlValue = val.into();

        unsafe {
            let val_ptr = NonNull::new(libcamera_control_value_create()).unwrap();
            ctrl_val.write(val_ptr);
            libcamera_control_list_set(self.ptr().cast_mut(), C::ID as _, val_ptr.as_ptr());
            libcamera_control_value_destroy(val_ptr.as_ptr());
        }

        Ok(())
    }
}

impl<'d> IntoIterator for &'d ControlList {
    type Item = (u32, ControlValue);

    type IntoIter = ControlListRefIterator<'d>;

    fn into_iter(self) -> Self::IntoIter {
        ControlListRefIterator {
            it: NonNull::new(unsafe { libcamera_control_list_iter(self.ptr().cast_mut()) }).unwrap(),
            _phantom: Default::default(),
        }
    }
}

impl core::fmt::Debug for ControlList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for (id, val) in self.into_iter() {
            match ControlId::try_from(id) {
                // Try to parse dynamic control, if not successful, just display the raw ControlValue
                Ok(id) => match controls::make_dyn(id, val.clone()) {
                    Ok(val) => map.entry(&id, &val),
                    Err(_) => map.entry(&id, &val),
                },
                // If ControlId is unknown just use u32 as key
                Err(_) => map.entry(&id, &val),
            };
        }
        map.finish()
    }
}

#[repr(transparent)]
pub struct PropertyList(libcamera_control_list_t);

impl PropertyList {
    pub(crate) unsafe fn from_ptr<'a>(ptr: NonNull<libcamera_control_list_t>) -> &'a mut Self {
        // Safety: we can cast it because of `#[repr(transparent)]`
        &mut *(ptr.as_ptr() as *mut Self)
    }

    pub(crate) fn ptr(&self) -> *const libcamera_control_list_t {
        // Safety: we can cast it because of `#[repr(transparent)]`
        &self.0 as *const libcamera_control_list_t
    }

    pub fn get<C: Property>(&self) -> Result<C, ControlError> {
        let val_ptr = NonNull::new(unsafe { libcamera_control_list_get(self.ptr().cast_mut(), C::ID as _).cast_mut() })
            .ok_or(ControlError::NotFound(C::ID))?;

        let val = unsafe { ControlValue::read(val_ptr) }?;
        Ok(C::try_from(val)?)
    }

    /// Sets property value.
    ///
    /// This can fail if property is not supported by the camera, but due to libcamera API limitations an error will not
    /// be returned. Use [PropertyList::get] if you need to ensure that value was set.
    pub fn set<C: Property>(&mut self, val: C) -> Result<(), ControlError> {
        let ctrl_val: ControlValue = val.into();

        unsafe {
            let val_ptr = NonNull::new(libcamera_control_value_create()).unwrap();
            ctrl_val.write(val_ptr);
            libcamera_control_list_set(self.ptr().cast_mut(), C::ID as _, val_ptr.as_ptr());
            libcamera_control_value_destroy(val_ptr.as_ptr());
        }

        Ok(())
    }
}

impl<'d> IntoIterator for &'d PropertyList {
    type Item = (u32, ControlValue);

    type IntoIter = ControlListRefIterator<'d>;

    fn into_iter(self) -> Self::IntoIter {
        ControlListRefIterator {
            it: NonNull::new(unsafe { libcamera_control_list_iter(self.ptr().cast_mut()) }).unwrap(),
            _phantom: Default::default(),
        }
    }
}

impl core::fmt::Debug for PropertyList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for (id, val) in self.into_iter() {
            match PropertyId::try_from(id) {
                // Try to parse dynamic property, if not successful, just display the raw ControlValue
                Ok(id) => match properties::make_dyn(id, val.clone()) {
                    Ok(val) => map.entry(&id, &val),
                    Err(_) => map.entry(&id, &val),
                },
                // If PropertyId is unknown just use u32 as key
                Err(_) => map.entry(&id, &val),
            };
        }
        map.finish()
    }
}

pub struct ControlListRefIterator<'d> {
    it: NonNull<libcamera_control_list_iter_t>,
    _phantom: PhantomData<&'d ()>,
}

impl Iterator for ControlListRefIterator<'_> {
    type Item = (u32, ControlValue);

    fn next(&mut self) -> Option<Self::Item> {
        if unsafe { libcamera_control_list_iter_end(self.it.as_ptr()) } {
            None
        } else {
            let id = unsafe { libcamera_control_list_iter_id(self.it.as_ptr()) };
            let val_ptr =
                NonNull::new(unsafe { libcamera_control_list_iter_value(self.it.as_ptr()).cast_mut() }).unwrap();
            let val = unsafe { ControlValue::read(val_ptr) }.unwrap();

            unsafe { libcamera_control_list_iter_next(self.it.as_ptr()) };

            Some((id, val))
        }
    }
}

impl Drop for ControlListRefIterator<'_> {
    fn drop(&mut self) {
        unsafe { libcamera_control_list_iter_destroy(self.it.as_ptr()) }
    }
}
